import ReactMarkdown, { type Components } from 'react-markdown';
import { openUrl } from '@tauri-apps/plugin-opener';

const WEB = /^https?:\/\//i;

/**
 * Release notes are text from the forge, so nothing in them may run or load:
 * raw HTML is dropped, images show only their description, and only web
 * links are followed, in the browser rather than this window.
 */
const components: Components = {
  a: ({ href, children }) =>
    href && WEB.test(href) ? (
      <a
        href={href}
        title={href}
        onClick={(e) => {
          e.preventDefault();
          openUrl(href);
        }}
      >
        {children}
      </a>
    ) : (
      <span>{children}</span>
    ),
  img: ({ alt }) => (alt ? <span className="markdown-img">[{alt}]</span> : null),
};

export default function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown components={components} skipHtml>
        {text}
      </ReactMarkdown>
    </div>
  );
}
