import { expect, test, type Page } from 'playwright/test';

const sessions = [
  {
    id: 'session-codex',
    agent: 'codex',
    title: 'Codex session',
    cwd: '/home/test/workbench/codex',
    status: 'running',
    created_at_ms: 1,
    exit_code: null,
    error: null,
    native_session_id: 'codex-native',
  },
  {
    id: 'session-claude',
    agent: 'claude',
    title: 'Claude session',
    cwd: '/home/test/workbench/claude',
    status: 'running',
    created_at_ms: 2,
    exit_code: null,
    error: null,
    native_session_id: 'claude-native',
  },
  {
    id: 'session-codex-secondary',
    agent: 'codex',
    title: 'Secondary Codex session',
    cwd: '/home/test/workbench/codex-secondary',
    status: 'running',
    created_at_ms: 3,
    exit_code: null,
    error: null,
    native_session_id: 'codex-secondary-native',
  },
];

const history = (id: string, title: string, cwd: string) => ({
  id,
  agent: 'codex',
  title,
  cwd,
  start_time: '2026-08-16 10:00:00',
  end_time: null,
  file_mtime: '2026-08-16 10:00:00',
  message_count: 4,
});

async function dragRowTo(page: Page, source: ReturnType<Page['locator']>, target: ReturnType<Page['locator']>) {
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  expect(sourceBox).not.toBeNull();
  expect(targetBox).not.toBeNull();
  await page.mouse.move(sourceBox!.x + sourceBox!.width / 2, sourceBox!.y + sourceBox!.height / 2);
  await page.mouse.down();
  await page.mouse.move(targetBox!.x + targetBox!.width / 2, targetBox!.y + targetBox!.height / 2, { steps: 12 });
  await page.mouse.up();
}

const workspace = {
  general_root: '/home/test/workbench',
  projects: [
    {
      project: { id: 'project-a', name: 'Project A', path: '/home/test/project-a', pinned: false, sort_order: 0 },
      history: [history('project-a-1', 'Project A first', '/home/test/project-a'), history('project-a-2', 'Project A second', '/home/test/project-a')],
    },
    {
      project: { id: 'project-b', name: 'Project B', path: '/home/test/project-b', pinned: false, sort_order: 1 },
      history: [],
    },
  ],
  general: [],
  other: [
    { path: '/home/test/other-a', available: true, items: [] },
    { path: '/home/test/other-b', available: true, items: [] },
  ],
};

async function mockBackend(page: Page) {
  await page.addInitScript(() => {
    const sockets = new Map<string, MockWebSocket>();
    Object.assign(window, { __akmuxSocketEvents: [], __akmuxReorders: [], __akmuxNotifications: [], __akmuxEmitBellOnResize: false });

    class MockNotification {
      static permission: NotificationPermission = 'granted';
      static async requestPermission() {
        return 'granted' as NotificationPermission;
      }

      constructor(title: string, options?: NotificationOptions) {
        (window as unknown as { __akmuxNotifications: Array<{ title: string; body?: string }> }).__akmuxNotifications.push({ title, body: options?.body });
      }
    }

    Object.defineProperty(window, 'Notification', { configurable: true, value: MockNotification });

    class MockWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readonly url: string;
      readyState = MockWebSocket.CONNECTING;
      binaryType: BinaryType = 'blob';

      constructor(url: string | URL) {
        super();
        this.url = String(url);
        sockets.set(this.url, this);
        window.setTimeout(() => {
          this.readyState = MockWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
          const prefix = this.url.includes('session-claude') ? 'claude' : this.url.includes('secondary') ? 'codex-secondary' : 'codex';
          const output = Array.from({ length: 200 }, (_, index) => `${prefix} history line ${index + 1}\r\n`).join('');
          (window as unknown as { __akmuxSocketEvents: Array<{ url: string; kind: string }> }).__akmuxSocketEvents.push({ url: this.url, kind: 'output' });
          this.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode(output).buffer }));
        }, 0);
      }

      send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
        if (typeof data !== 'string') return;
        try {
          const message = JSON.parse(data) as { type?: string; rows?: number; cols?: number };
          if (message.type !== 'resize') return;
          (window as unknown as { __akmuxSocketEvents: Array<{ url: string; kind: string; rows?: number; cols?: number }> }).__akmuxSocketEvents.push({
            url: this.url,
            kind: 'resize',
            rows: message.rows,
            cols: message.cols,
          });
          if ((window as unknown as { __akmuxEmitBellOnResize: boolean }).__akmuxEmitBellOnResize) {
            this.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode('\x07').buffer }));
          }
        } catch {
          // Terminal input is binary and does not participate in these assertions.
        }
      }

      close() {
        this.readyState = MockWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { configurable: true, value: MockWebSocket });
    Object.assign(window, {
      __akmuxEmitOutput: (sessionId: string, text: string) => {
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode(text).buffer }));
      },
      __akmuxEmitStatus: (session: unknown) => {
        const sessionId = (session as { id: string }).id;
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'status', session }) }));
      },
    });
  });

  await page.route('**/api/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === '/api/workspaces') {
      await route.fulfill({ json: workspace });
      return;
    }
    if (url.pathname === '/api/settings') {
      await route.fulfill({
        json: {
          general_root: '/home/test/workbench',
          projects: workspace.projects.map(group => group.project),
          other_directories: workspace.other.map((group, index) => ({ path: group.path, pinned: false, last_opened_ms: index, sort_order: index })),
          project_sort: 'manual',
          general_sort: 'recent',
          other_sort: 'manual',
          directory_sort: {},
          session_order: {},
        },
      });
      return;
    }
    if (url.pathname === '/api/sessions' && request.method() === 'GET') {
      await route.fulfill({ json: sessions });
      return;
    }
    if (url.pathname === '/api/directories') {
      await route.fulfill({ json: { path: '/home/test', parent: '/home', home: '/home/test', entries: [{ name: 'workbench', path: '/home/test/workbench' }] } });
      return;
    }
    if (request.method() === 'DELETE') {
      await route.fulfill({ status: 204 });
      return;
    }
    if (url.pathname === '/api/reorder') {
      const body = request.postDataJSON();
      await page.evaluate(value => {
        (window as unknown as { __akmuxReorders: unknown[] }).__akmuxReorders.push(value);
      }, body);
      await route.fulfill({ json: {} });
      return;
    }
    if (url.pathname.endsWith('/details')) {
      await route.fulfill({
        json: {
          managed_session_id: 'session-codex',
          native_session_id: 'codex-native',
          agent: 'codex',
          provider_id: 'openai',
          provider_name: 'OpenAI',
          profile_id: null,
          model: 'gpt-5',
          prompt_tokens: 120,
          completion_tokens: 40,
          cache_read_tokens: 30,
          cache_creation_tokens: 0,
          message_count: 8,
        },
      });
      return;
    }
    await route.fulfill({ status: 204 });
  });
}

test.beforeEach(async ({ page }) => {
  await mockBackend(page);
  await page.goto('/');
  await expect(page.locator('[data-session-tab]')).toHaveCount(3);
});

test('settings expose full acrylic range and terminal font size', async ({ page }) => {
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  const acrylic = page.getByRole('slider', { name: 'Acrylic transparency' });
  await expect(acrylic).toHaveAttribute('min', '0');
  await expect(acrylic).toHaveAttribute('max', '100');
  const terminalFontSize = page.getByRole('slider', { name: 'Session font size' });
  await expect(terminalFontSize).toHaveAttribute('min', '10');
  await expect(terminalFontSize).toHaveValue('12');
  const controls = page.locator('.segmented-control');
  await expect(controls).toHaveCount(2);
  const layouts = await controls.evaluateAll(elements =>
    elements.map(element => ({ columns: getComputedStyle(element).gridTemplateColumns.split(' ').length, width: element.getBoundingClientRect().width })),
  );
  expect(layouts[0]).toEqual(expect.objectContaining({ columns: 3 }));
  expect(layouts[1]).toEqual(expect.objectContaining({ columns: 2 }));
  const themeOverflows = await controls
    .first()
    .locator('button')
    .evaluateAll(buttons => buttons.some(button => button.scrollWidth > button.clientWidth));
  expect(themeOverflows).toBe(false);
  await expect(controls.nth(1).locator('button').first()).toHaveCSS('padding-left', '16px');
});

test('directory picker starts at home without opening the hidden-directory tooltip', async ({ page }) => {
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('button', { name: 'Browse' }).click();
  await expect(page.locator('input[name="akmux-directory-path"]')).toHaveValue('/home/test');
  await page.waitForTimeout(650);
  await expect(page.getByText('Show hidden directories', { exact: true })).toBeHidden();
});

test('directory picker replaces backend errors with a recoverable GUI state', async ({ page }) => {
  await page.route('**/api/directories**', route => route.fulfill({ status: 400, json: { error: 'No such file or directory: /private/path' } }));
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('button', { name: 'Browse' }).click();
  await expect(page.getByText('Directory unavailable', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry' })).toBeVisible();
  await expect(page.getByText(/No such file|private\/path/)).toHaveCount(0);
});

test('directory picker shows a service fallback when the backend is unreachable', async ({ page }) => {
  await page.route('**/api/directories**', route => route.abort('connectionrefused'));
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('button', { name: 'Browse' }).click();
  await expect(page.getByText('Service unavailable', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry' })).toBeVisible();
});

test('settings and session details do not expose raw backend errors', async ({ page }) => {
  await page.route('**/api/settings', route => {
    if (route.request().method() === 'PATCH') {
      return route.fulfill({ status: 500, json: { error: 'Database lock poisoned at /private/akmux.db' } });
    }
    return route.fallback();
  });
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Service unavailable', { exact: true })).toBeVisible();
  await expect(page.getByText(/Database lock|private\/akmux/)).toHaveCount(0);
  await page.getByRole('button', { name: 'Cancel' }).click();

  await page.evaluate(session => {
    (window as unknown as { __akmuxEmitStatus: (session: unknown) => void }).__akmuxEmitStatus({
      ...session,
      status: 'error',
      error: 'Failed to spawn /private/bin/codex: permission denied',
    });
  }, sessions[0]);
  await page.getByRole('button', { name: 'Session details' }).click();
  await expect(page.getByText('Session could not continue', { exact: true })).toBeVisible();
  await expect(page.getByText(/private\/bin|permission denied/)).toHaveCount(0);
});

test('dialogs do not automatically focus text inputs', async ({ page }) => {
  await page.locator('#search-button').click();
  await expect(page.locator('#search-popover input')).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.activeElement?.tagName)).not.toBe('INPUT');
});

test('terminal synchronizes its fitted size before buffered Claude output arrives', async ({ page }) => {
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __akmuxSocketEvents: Array<{ url: string; kind: string }> }).__akmuxSocketEvents
          .filter(event => event.url.includes('session-claude'))
          .map(event => event.kind),
      ),
    )
    .toEqual(expect.arrayContaining(['resize', 'output']));
  const firstClaudeEvent = await page.evaluate(() =>
    (window as unknown as { __akmuxSocketEvents: Array<{ url: string; kind: string }> }).__akmuxSocketEvents.find(event => event.url.includes('session-claude'))?.kind,
  );
  expect(firstClaudeEvent).toBe('resize');
});

test('terminal output can be selected and copied', async ({ page }) => {
  const screen = page.locator('.terminal-host[aria-hidden="false"] .xterm-screen');
  await expect(screen).toBeVisible();
  const box = await screen.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + 12, box!.y + 12);
  await page.mouse.down();
  await page.mouse.move(box!.x + 230, box!.y + 12, { steps: 8 });
  await page.mouse.up();
  await page.keyboard.press('Control+c');
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('history line');
});

test('manual ordering exposes drag affordances and persists workspace order', async ({ page }) => {
  await page.evaluate(() => {
    document.addEventListener('dragstart', event => event.dataTransfer?.clearData());
  });
  await page.getByText('Workspaces', { exact: true }).click();
  const projectGroup = page.locator('[data-project-group="project-a"]');
  const projectA = page.locator('[data-project-row="project-a"]');
  const projectB = page.locator('[data-project-row="project-b"]');
  await expect(projectA.locator('[data-drag-handle]')).toHaveCount(0);
  await dragRowTo(page, projectB, projectA);
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __akmuxReorders: unknown[] }).__akmuxReorders))
    .toContainEqual({ kind: 'projects', scope: 'projects', ids: ['project-b', 'project-a'] });

  await projectA.locator('button[title="/home/test/project-a"]').click();
  const projectSessions = projectGroup.locator('.history-row');
  await expect(projectSessions).toHaveCount(2);
  await expect(projectSessions.nth(0).locator('[data-drag-handle]')).toHaveCount(0);
  await dragRowTo(page, projectSessions.nth(1), projectSessions.nth(0));
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __akmuxReorders: unknown[] }).__akmuxReorders))
    .toContainEqual({ kind: 'sessions', scope: 'project:project-a', ids: ['codex:project-a-2', 'codex:project-a-1'] });

  await page.getByText('Other directories', { exact: true }).click();
  const otherA = page.locator('[data-directory-row="/home/test/other-a"]');
  const otherB = page.locator('[data-directory-row="/home/test/other-b"]');
  await expect(otherA.locator('[data-drag-handle]')).toHaveCount(0);
  await dragRowTo(page, otherB, otherA);
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __akmuxReorders: unknown[] }).__akmuxReorders))
    .toContainEqual({ kind: 'directories', scope: 'other', ids: ['/home/test/other-b', '/home/test/other-a'] });
});

test('desktop controls do not expose browser focus outlines', async ({ page }) => {
  const search = page.locator('#search-button');
  await search.focus();
  await expect(search).toHaveCSS('outline-style', 'none');
  await expect(search).toHaveCSS('box-shadow', 'none');
});

test('sidebar resizing does not create Codex attention signals', async ({ page }) => {
  await page.locator('[data-session-tab="session-claude"]').click();
  await page.evaluate(() => {
    (window as unknown as { __akmuxEmitBellOnResize: boolean }).__akmuxEmitBellOnResize = true;
  });
  const handle = page.locator('[data-sidebar-resize-handle]');
  const box = await handle.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + 100);
  await page.mouse.down();
  await page.mouse.move(box!.x + 48, box!.y + 100, { steps: 5 });
  await page.mouse.up();
  await expect(page.locator('[data-session-tab="session-codex"] .session-signal')).toHaveCount(0);
});

test('background interaction and completion events create system notifications', async ({ page }) => {
  await page.evaluate(() => {
    Object.defineProperty(document, 'hasFocus', { configurable: true, value: () => false });
    (window as unknown as { __akmuxEmitOutput: (sessionId: string, text: string) => void }).__akmuxEmitOutput('session-codex', 'Permission required to continue');
    (window as unknown as { __akmuxEmitStatus: (session: unknown) => void }).__akmuxEmitStatus({
      ...{
        id: 'session-claude',
        agent: 'claude',
        title: 'Claude session',
        cwd: '/home/test/workbench/claude',
        created_at_ms: 2,
        native_session_id: 'claude-native',
      },
      status: 'exited',
      exit_code: 0,
      error: null,
    });
  });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __akmuxNotifications: Array<{ title: string }> }).__akmuxNotifications.map(item => item.title)))
    .toEqual(expect.arrayContaining(['Session needs attention', 'Session finished']));
});

test('closing the active session focuses the adjacent session and terminal fits its host', async ({ page }) => {
  await expect(page.locator('[data-session-tab="session-codex"]')).toHaveAttribute('data-active', 'true');
  await page.locator('[data-session-tab="session-codex-secondary"]').click();
  await expect(page.locator('[data-session-tab="session-codex-secondary"]')).toHaveAttribute('data-active', 'true');
  await page.locator('[data-session-tab="session-codex"]').click();
  await page.getByRole('button', { name: 'Close session' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'Close session' }).click();
  await expect(page.locator('[data-session-tab="session-claude"]')).toHaveAttribute('data-active', 'true');
  await page.locator('[data-session-tab="session-codex-secondary"]').click();
  await expect(page.locator('[data-session-tab="session-codex-secondary"]')).toHaveAttribute('data-active', 'true');

  await expect
    .poll(() =>
      page.locator('.terminal-host[aria-hidden="false"]').evaluate(host => {
        const terminal = host.querySelector<HTMLElement>('.xterm');
        const screen = host.querySelector<HTMLElement>('.xterm-screen');
        if (!terminal || !screen) return Number.POSITIVE_INFINITY;
        return terminal.getBoundingClientRect().width - screen.getBoundingClientRect().width;
      }),
    )
    .toBeLessThan(20);

  const host = page.locator('.terminal-host[aria-hidden="false"]');
  await expect
    .poll(() =>
      host.evaluate(element => {
        const track = element.querySelector<HTMLElement>('.scrollbar.vertical');
        const slider = track?.querySelector<HTMLElement>('.slider');
        if (!track || !slider) return 0;
        return track.getBoundingClientRect().height - slider.getBoundingClientRect().height;
      }),
    )
    .toBeGreaterThan(100);
  const slider = host.locator('.scrollbar.vertical .slider');
  const initialTop = await slider.evaluate(element => element.getBoundingClientRect().top);
  await host.hover();
  await page.mouse.wheel(0, -2_000);
  await expect.poll(() => slider.evaluate(element => element.getBoundingClientRect().top)).toBeLessThan(initialTop - 5);
});
