// Inline dog SVG — same paths as design/mockup_main_shell.html <symbol id="dog">
function DogMark() {
  return (
    <svg className="mark" viewBox="0 0 64 64">
      <ellipse cx="32" cy="49" rx="17" ry="12" fill="#E3A03A" />
      <circle cx="32" cy="29" r="16" fill="#E9A845" />
      <path d="M17 19 Q11 27 17 35 Q24 31 24 22 Z" fill="#C8842B" />
      <path d="M47 19 Q53 27 47 35 Q40 31 40 22 Z" fill="#C8842B" />
      <ellipse cx="32" cy="34" rx="9" ry="7" fill="#F6DCA9" />
      <circle cx="26.5" cy="27" r="2.2" fill="#2A211A" />
      <circle cx="37.5" cy="27" r="2.2" fill="#2A211A" />
      <ellipse cx="32" cy="32" rx="2.6" ry="2" fill="#2A211A" />
      <path
        d="M32 34 Q32 37 29.5 37 M32 34 Q32 37 34.5 37"
        stroke="#2A211A"
        strokeWidth="1.2"
        fill="none"
        strokeLinecap="round"
      />
    </svg>
  );
}

interface Props {
  folderPath:     string | null;
  onFolderSelect: () => void;
  onSettingsOpen: () => void;
}

export function Topbar({ folderPath, onFolderSelect, onSettingsOpen }: Props) {
  const displayPath = folderPath
    ? folderPath.replace(/^\/Users\/[^/]+/, "~")
    : null;

  return (
    <div className="topbar">
      <div className="brand">
        <DogMark />
        TidyDog
      </div>
      <div className="folder-pill" onClick={onFolderSelect}>
        {displayPath ? (
          <>정리 대상 <b>{displayPath}</b> ▾</>
        ) : (
          <>폴더 선택 ▾</>
        )}
      </div>
      <div className="spacer" />
      <div className="icon-btn" title="설정" onClick={onSettingsOpen}
        style={{ cursor: "pointer" }}>⚙</div>
    </div>
  );
}
