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
    Object.assign(window, {
      __akmuxSocketEvents: [],
      __akmuxReorders: [],
      __akmuxNotifications: [],
      __akmuxOpenedUrls: [],
      __akmuxEmitBellOnResize: false,
      __akmuxReplayApprovalOnConnect: false,
      __akmuxServerClockOffsetMs: 0,
    });
    Object.defineProperty(window, 'open', {
      configurable: true,
      value: (url?: string | URL) => {
        if (url) (window as unknown as { __akmuxOpenedUrls: string[] }).__akmuxOpenedUrls.push(String(url));
        return null;
      },
    });

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
          const replayApproval = (window as unknown as { __akmuxReplayApprovalOnConnect: boolean }).__akmuxReplayApprovalOnConnect && prefix === 'codex';
          const deliverBootstrap = () => {
            const output = `${replayApproval ? '\x1b]9;approval requested\x07' : ''}${Array.from({ length: 200 }, (_, index) => `${prefix} history line ${index + 1}\r\n`).join('')}`;
            (window as unknown as { __akmuxSocketEvents: Array<{ url: string; kind: string }> }).__akmuxSocketEvents.push({ url: this.url, kind: 'output' });
            this.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode(output).buffer }));
            const sessionId = prefix === 'claude' ? 'session-claude' : prefix === 'codex-secondary' ? 'session-codex-secondary' : 'session-codex';
            this.dispatchEvent(
              new MessageEvent('message', {
                data: JSON.stringify({
                  type: 'status',
                  server_time_ms: Date.now() + (window as unknown as { __akmuxServerClockOffsetMs: number }).__akmuxServerClockOffsetMs,
                  session: {
                    id: sessionId,
                    agent: prefix === 'claude' ? 'claude' : 'codex',
                    title: `${prefix} session`,
                    cwd: `/home/test/${prefix}`,
                    status: 'running',
                    created_at_ms: 1,
                    exit_code: null,
                    error: null,
                    native_session_id: `${prefix}-native`,
                  },
                }),
              }),
            );
          };
          window.setTimeout(deliverBootstrap, replayApproval ? 150 : 0);
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
      __akmuxEmitBell: (sessionId: string) => {
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode('\x07').buffer }));
      },
      __akmuxEmitAttention: (sessionId: string, kind: 'input' | 'completed', occurredAtMs?: number) => {
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'attention', kind, occurred_at_ms: occurredAtMs }) }));
      },
      __akmuxReconnectWithApprovalScrollback: (sessionId: string) => {
        (window as unknown as { __akmuxReplayApprovalOnConnect: boolean }).__akmuxReplayApprovalOnConnect = true;
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.close();
      },
      __akmuxReconnectWithClockOffset: (sessionId: string, offsetMs: number) => {
        (window as unknown as { __akmuxServerClockOffsetMs: number }).__akmuxServerClockOffsetMs = offsetMs;
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.close();
      },
      __akmuxEmitCodexApproval: (sessionId: string) => {
        const socket = [...sockets.entries()].find(([url]) => url.includes(sessionId))?.[1];
        socket?.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode('\x1b]9;approval requested\x07').buffer }));
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

test('settings expose the full material transparency range and terminal font size', async ({ page }) => {
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByText('Background material', { exact: true })).toBeVisible();
  const material = page.getByRole('slider', { name: 'Material transparency' });
  await expect(material).toHaveAttribute('min', '0');
  await expect(material).toHaveAttribute('max', '100');
  await expect(material).toHaveValue('30');
  const materialEndpoints = await page.locator('.app-background').evaluate(element => {
    const rgba = (color: string) => {
      const canvas = document.createElement('canvas');
      canvas.width = 1;
      canvas.height = 1;
      const context = canvas.getContext('2d')!;
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      return [...context.getImageData(0, 0, 1, 1).data];
    };
    document.documentElement.dataset.desktopShell = 'true';
    document.documentElement.dataset.acrylic = 'true';
    document.documentElement.dataset.theme = 'dark';
    document.documentElement.style.setProperty('--material-tint-opacity', '100%');
    const opaque = rgba(getComputedStyle(element).backgroundColor);
    document.documentElement.style.setProperty('--material-tint-opacity', '0%');
    const transparent = rgba(getComputedStyle(element).backgroundColor);
    return { opaque, transparent };
  });
  expect(materialEndpoints).toEqual({ opaque: [9, 11, 10, 255], transparent: [0, 0, 0, 0] });
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

test('terminal hyperlinks leave the application through the external URL handler', async ({ page }) => {
  await page.evaluate(() => {
    const output = (window as unknown as { __akmuxEmitOutput: (sessionId: string, text: string) => void }).__akmuxEmitOutput;
    output('session-codex', '\r\n\x1b]8;;https://example.com/docs\x07Open external docs\x1b]8;;\x07\r\n');
  });
  const linkRow = page.locator('.terminal-host[aria-hidden="false"] .xterm-rows > div').filter({ hasText: 'Open external docs' });
  await expect(linkRow).toBeVisible();
  const box = await linkRow.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.click(box!.x + 60, box!.y + box!.height / 2, { modifiers: ['Control'] });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __akmuxOpenedUrls: string[] }).__akmuxOpenedUrls))
    .toEqual(['https://example.com/docs']);
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

test('sidebar expansion is restored for the active backend', async ({ page }) => {
  await page.getByText('Workspaces', { exact: true }).click();
  await page.locator('[data-project-row="project-a"] button[title="/home/test/project-a"]').click();
  await expect(page.locator('[data-project-group="project-a"] .history-row')).toHaveCount(2);

  await page.reload();

  await expect(page.locator('[data-project-group="project-a"] .history-row')).toHaveCount(2);
});

test('the active session is restored without briefly overwriting its backend key', async ({ page }) => {
  await page.locator('[data-session-tab="session-codex-secondary"]').click();
  await page.reload();
  await expect(page.locator('[data-session-tab="session-codex-secondary"]')).toHaveAttribute('data-active', 'true');
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

test('reconnecting does not treat Codex approval sequences in scrollback as new attention', async ({ page }) => {
  await page.locator('[data-session-tab="session-claude"]').click();
  await page.evaluate(() => {
    const reconnect = (window as unknown as { __akmuxReconnectWithApprovalScrollback: (sessionId: string) => void }).__akmuxReconnectWithApprovalScrollback;
    reconnect('session-codex');
  });
  await page.waitForTimeout(1_500);
  await expect(page.locator('[data-session-tab="session-codex"]')).not.toHaveAttribute('data-attention', /.+/);
});

test('stale structured attention delayed by suspension is ignored', async ({ page }) => {
  await page.locator('[data-session-tab="session-claude"]').click();
  await page.evaluate(() => {
    const attention = (
      window as unknown as { __akmuxEmitAttention: (sessionId: string, kind: 'input' | 'completed', occurredAtMs?: number) => void }
    ).__akmuxEmitAttention;
    attention('session-codex', 'completed', Date.now() - 120_000);
  });
  await expect(page.locator('[data-session-tab="session-codex"]')).not.toHaveAttribute('data-attention', /.+/);
});

test('fresh remote attention survives backend clock skew', async ({ page }) => {
  await page.locator('[data-session-tab="session-claude"]').click();
  await page.evaluate(() => {
    const reconnect = (
      window as unknown as { __akmuxReconnectWithClockOffset: (sessionId: string, offsetMs: number) => void }
    ).__akmuxReconnectWithClockOffset;
    reconnect('session-codex', -300_000);
  });
  await page.waitForTimeout(1_500);
  await page.evaluate(() => {
    const attention = (
      window as unknown as { __akmuxEmitAttention: (sessionId: string, kind: 'input' | 'completed', occurredAtMs?: number) => void }
    ).__akmuxEmitAttention;
    attention('session-codex', 'completed', Date.now() - 300_000);
  });
  await expect(page.locator('[data-session-tab="session-codex"]')).toHaveAttribute('data-attention', 'completed');
});

test('structured permission and completion events create deduplicated system notifications', async ({ page }) => {
  await page.waitForTimeout(800);
  await page.evaluate(() => {
    Object.defineProperty(document, 'hasFocus', { configurable: true, value: () => false });
    const approval = (window as unknown as { __akmuxEmitCodexApproval: (sessionId: string) => void }).__akmuxEmitCodexApproval;
    const attention = (window as unknown as { __akmuxEmitAttention: (sessionId: string, kind: 'input' | 'completed') => void }).__akmuxEmitAttention;
    approval('session-codex');
    approval('session-codex');
    attention('session-claude', 'completed');
    attention('session-claude', 'completed');
  });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __akmuxNotifications: Array<{ title: string }> }).__akmuxNotifications.map(item => item.title)))
    .toEqual(['Session needs attention', 'Response completed']);
});

test('inactive sessions retain permission and completion signals in the application', async ({ page }) => {
  await page.waitForTimeout(800);
  await page.locator('[data-session-tab="session-claude"]').click();
  await page.evaluate(() => {
    const attention = (window as unknown as { __akmuxEmitAttention: (sessionId: string, kind: 'input' | 'completed') => void }).__akmuxEmitAttention;
    attention('session-codex', 'completed');
  });
  await expect(page.locator('[data-session-tab="session-codex"]')).toHaveAttribute('data-attention', 'completed');
  await expect(page.locator('[data-session-tab="session-codex"] .session-signal-completed')).toBeVisible();

  await page.locator('[data-session-tab="session-codex"]').click();
  await expect(page.locator('[data-session-tab="session-codex"]')).not.toHaveAttribute('data-attention', /.+/);
  await page.locator('[data-session-tab="session-claude"]').click();
  await page.evaluate(() => {
    const bell = (window as unknown as { __akmuxEmitBell: (sessionId: string) => void }).__akmuxEmitBell;
    bell('session-codex');
  });
  await page.waitForTimeout(100);
  await expect(page.locator('[data-session-tab="session-codex"]')).not.toHaveAttribute('data-attention', /.+/);
  await page.evaluate(() => {
    const approval = (window as unknown as { __akmuxEmitCodexApproval: (sessionId: string) => void }).__akmuxEmitCodexApproval;
    approval('session-codex');
  });
  await expect(page.locator('[data-session-tab="session-codex"]')).toHaveAttribute('data-attention', 'input');

  await page.locator('[data-session-tab="session-codex"]').click();
  await page.evaluate(() => {
    const attention = (window as unknown as { __akmuxEmitAttention: (sessionId: string, kind: 'input' | 'completed') => void }).__akmuxEmitAttention;
    attention('session-claude', 'completed');
  });
  await expect(page.locator('[data-session-tab="session-claude"]')).toHaveAttribute('data-attention', 'completed');
  await expect(page.locator('[data-session-tab="session-claude"] .session-signal-completed')).toBeVisible();
});

test('an exited session is removed even when Ctrl+C returns a nonzero exit code', async ({ page }) => {
  await page.evaluate(() => {
    (window as unknown as { __akmuxEmitStatus: (session: unknown) => void }).__akmuxEmitStatus({
      id: 'session-codex',
      agent: 'codex',
      title: 'Codex session',
      cwd: '/home/test/workbench/codex',
      status: 'exited',
      created_at_ms: 1,
      exit_code: 130,
      error: null,
      native_session_id: 'codex-native',
    });
  });
  await expect(page.locator('[data-session-tab="session-codex"]')).toHaveCount(0, { timeout: 2_000 });
  await expect(page.locator('[data-session-tab="session-claude"]')).toHaveAttribute('data-active', 'true');
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
