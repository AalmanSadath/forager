import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
const { default: Markdown } = await import('./Markdown');

describe('Markdown', () => {
  it('formats notes but never runs or loads anything from them', () => {
    const html = renderToStaticMarkup(
      <Markdown
        text={[
          '## Fixed',
          '- a **bug**',
          '<script>alert(1)</script><img src="https://x/a.png">',
          '![diagram](https://x/d.png) [site](https://example.org) [local](./news.txt) [bad](javascript:alert(1))',
        ].join('\n')}
      />,
    );
    expect(html).toContain('<h2>Fixed</h2>');
    expect(html).toContain('<strong>bug</strong>');
    expect(html).not.toContain('<script');
    expect(html).not.toContain('<img');
    expect(html).toContain('[diagram]');
    expect(html).toContain('href="https://example.org"');
    expect(html).not.toContain('./news.txt"');
    expect(html).not.toContain('javascript:');
  });
});
