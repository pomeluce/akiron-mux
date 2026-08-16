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

async function mockBackend(page: Page) {
  await page.addInitScript(() => {
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
        window.setTimeout(() => {
          this.readyState = MockWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
          const prefix = this.url.includes('session-claude') ? 'claude' : this.url.includes('secondary') ? 'codex-secondary' : 'codex';
          const output = Array.from({ length: 200 }, (_, index) => `${prefix} history line ${index + 1}\r\n`).join('');
          this.dispatchEvent(new MessageEvent('message', { data: new TextEncoder().encode(output).buffer }));
        }, 0);
      }

      send() {}

      close() {
        this.readyState = MockWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { configurable: true, value: MockWebSocket });
  });

  await page.route('**/api/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === '/api/workspaces') {
      await route.fulfill({ json: { general_root: '/home/test/workbench', projects: [], general: [], other: [] } });
      return;
    }
    if (url.pathname === '/api/settings') {
      await route.fulfill({
        json: {
          general_root: '/home/test/workbench',
          projects: [],
          other_directories: [],
          project_sort: 'priority',
          general_sort: 'recent',
          other_sort: 'recent',
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
