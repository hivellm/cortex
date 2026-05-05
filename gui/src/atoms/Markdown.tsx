import { useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";

type MarkdownProps = {
  source: string;
  className?: string;
};

marked.setOptions({
  gfm: true,
  breaks: false,
});

export function Markdown({ source, className }: MarkdownProps) {
  const html = useMemo(() => {
    const raw = marked.parse(source ?? "", { async: false }) as string;
    return DOMPurify.sanitize(raw, {
      ADD_ATTR: ["target", "rel"],
    });
  }, [source]);

  return (
    <div
      className={`markdown ${className ?? ""}`.trim()}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
