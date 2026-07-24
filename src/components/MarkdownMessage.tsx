import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

// 안전 속성:
//  - raw HTML 미렌더: rehype-raw / dangerouslySetInnerHTML 미사용.
//    react-markdown 기본값은 문자열 내 <script>·<img onerror=…> 를 파싱하지 않고 텍스트로 처리한다.
//  - 위험 스킴 링크 차단: 기본 urlTransform 이 javascript:·data: 등을 무력화하며,
//    아래 커스텀 a 컴포넌트가 http/https 외 href 를 클릭 불가한 평문(span)으로 렌더한다(이중 방어).

function isSafeHref(href: unknown): href is string {
  return typeof href === "string" && /^https?:\/\//i.test(href);
}

// 디자인 토큰 매핑 — 빨강 없음. 모노는 IBM Plex Mono(--mono).
const components: Components = {
  h1: ({ children }) => (
    <div style={{ fontSize: 17, fontWeight: 700, color: "var(--ink)", margin: "12px 0 6px" }}>{children}</div>
  ),
  h2: ({ children }) => (
    <div style={{ fontSize: 15.5, fontWeight: 700, color: "var(--ink)", margin: "12px 0 6px" }}>{children}</div>
  ),
  h3: ({ children }) => (
    <div style={{ fontSize: 14.5, fontWeight: 700, color: "var(--primary)", margin: "10px 0 4px" }}>{children}</div>
  ),
  p: ({ children }) => <p style={{ margin: "6px 0", lineHeight: 1.6 }}>{children}</p>,
  strong: ({ children }) => <strong style={{ fontWeight: 700, color: "var(--ink)" }}>{children}</strong>,
  em: ({ children }) => <em style={{ fontStyle: "italic" }}>{children}</em>,
  ul: ({ children }) => <ul style={{ margin: "6px 0", paddingLeft: 20, lineHeight: 1.6 }}>{children}</ul>,
  ol: ({ children }) => <ol style={{ margin: "6px 0", paddingLeft: 22, lineHeight: 1.6 }}>{children}</ol>,
  li: ({ children }) => <li style={{ margin: "2px 0" }}>{children}</li>,
  hr: () => <hr style={{ border: "none", borderTop: "1px solid var(--line)", margin: "10px 0" }} />,
  blockquote: ({ children }) => (
    <blockquote style={{
      margin: "6px 0", padding: "2px 12px", borderLeft: "3px solid var(--line)",
      color: "var(--muted)",
    }}>{children}</blockquote>
  ),
  code: ({ className, children }) => {
    // 코드블록(펜스)은 className(language-*)을 가지고, 인라인 코드는 없다.
    const isBlock = typeof className === "string" && className.includes("language-");
    if (isBlock) {
      return (
        <code style={{ fontFamily: "var(--mono)", fontSize: 12.5, lineHeight: 1.5 }}>{children}</code>
      );
    }
    return (
      <code style={{
        fontFamily: "var(--mono)", fontSize: 12.5,
        background: "var(--surface)", border: "1px solid var(--line)",
        borderRadius: 4, padding: "1px 5px",
      }}>{children}</code>
    );
  },
  pre: ({ children }) => (
    <pre style={{
      margin: "8px 0", padding: "10px 12px",
      background: "var(--surface)", border: "1px solid var(--line)",
      borderRadius: "var(--r-sm)", overflowX: "auto",
      fontFamily: "var(--mono)", fontSize: 12.5,
    }}>{children}</pre>
  ),
  a: ({ href, children }) =>
    isSafeHref(href) ? (
      <a
        href={href}
        target="_blank"
        rel="noopener noreferrer nofollow"
        style={{ color: "var(--primary)", textDecoration: "underline" }}
      >
        {children}
      </a>
    ) : (
      // 위험 스킴(javascript:·data:) / 상대 링크 → 클릭 불가 평문.
      <span style={{ color: "var(--muted)" }}>{children}</span>
    ),
  table: ({ children }) => (
    <table style={{ borderCollapse: "collapse", margin: "8px 0", fontSize: 13 }}>{children}</table>
  ),
  th: ({ children }) => (
    <th style={{ border: "1px solid var(--line)", padding: "4px 8px", textAlign: "left", background: "var(--surface)" }}>{children}</th>
  ),
  td: ({ children }) => (
    <td style={{ border: "1px solid var(--line)", padding: "4px 8px" }}>{children}</td>
  ),
};

export function MarkdownMessage({ text }: { text: string }) {
  return (
    <div className="md-body">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}
