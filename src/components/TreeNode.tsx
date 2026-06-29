import { useState } from "react";
import { FileNode } from "../types";

function getIcon(node: FileNode): string {
  if (node.is_dir) return "📁";
  const ext = (node.ext ?? "").toLowerCase();
  if (["pdf", "hwpx", "hwp", "docx", "doc"].includes(ext)) return "📄";
  if (["png", "jpg", "jpeg", "gif", "webp", "heic", "svg"].includes(ext)) return "🖼";
  if (["dmg", "pkg", "exe", "zip", "tar", "gz", "7z"].includes(ext)) return "📦";
  if (["txt", "md", "rtf"].includes(ext)) return "🗒";
  if (["mp4", "mov", "avi", "mkv", "m4v"].includes(ext)) return "🎬";
  if (["mp3", "wav", "aac", "flac", "m4a"].includes(ext)) return "🎵";
  return "📄";
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}K`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)}M`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)}G`;
}

interface Props {
  node: FileNode;
  depth: number;
}

export function TreeNode({ node, depth }: Props) {
  const [isOpen, setIsOpen] = useState(depth === 0);

  // Base padding 8px + 18px per depth level
  const extraIndent = depth * 18;

  const toggle = node.is_dir ? (isOpen ? "▾" : "▸") : "";
  const sizeLabel = node.is_dir
    ? String(node.children.length)
    : node.size !== undefined
    ? formatSize(node.size)
    : "";

  return (
    <>
      <div
        className="node"
        style={{ paddingLeft: `${8 + extraIndent}px` }}
        onClick={() => node.is_dir && setIsOpen((o) => !o)}
      >
        <span className="tw">{toggle}</span>
        <span className="ic">{getIcon(node)}</span>
        <span className="nm">{node.name}</span>
        <span className="sz">{sizeLabel}</span>
      </div>
      {node.is_dir &&
        isOpen &&
        node.children.map((child) => (
          <TreeNode key={child.path} node={child} depth={depth + 1} />
        ))}
    </>
  );
}
