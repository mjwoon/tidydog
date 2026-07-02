import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Props {
  onClose: () => void;
}

export function SettingsModal({ onClose }: Props) {
  const [apiKey, setApiKey]     = useState("");
  const [saving, setSaving]     = useState(false);
  const [saved, setSaved]       = useState(false);
  const [error, setError]       = useState<string | null>(null);

  async function handleSave() {
    const trimmed = apiKey.trim();
    if (!trimmed) return;
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      await invoke("set_api_key", { key: trimmed });
      setApiKey(""); // N2: 제출 즉시 state 정리 (성공)
      setSaved(true);
      setTimeout(onClose, 900);
    } catch (err) {
      setApiKey(""); // N2: 실패해도 키는 state에서 즉시 제거 — 재입력 필요
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") handleSave();
    if (e.key === "Escape") onClose();
  }

  return (
    <div style={{
      position: "fixed", inset: 0, zIndex: 200,
      background: "rgba(32,39,42,0.45)",
      display: "flex", alignItems: "center", justifyContent: "center",
    }}
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div style={{
        background: "var(--surface)",
        borderRadius: "var(--r)",
        boxShadow: "0 4px 32px rgba(20,40,35,.18)",
        width: 420, maxWidth: "92vw",
        display: "flex", flexDirection: "column",
        overflow: "hidden",
      }}>
        {/* header */}
        <div style={{
          padding: "18px 22px 14px",
          borderBottom: "1px solid var(--line)",
          display: "flex", alignItems: "center", gap: 10,
        }}>
          <div style={{ flex: 1, fontWeight: 700, fontSize: 15 }}>설정</div>
          <button
            onClick={onClose}
            style={{
              background: "none", border: "none", cursor: "pointer",
              fontSize: 18, color: "var(--muted)", lineHeight: 1, padding: 4,
            }}
          >×</button>
        </div>

        {/* body */}
        <div style={{ padding: "18px 22px 22px" }}>
          <div style={{ marginBottom: 6, fontSize: 13, fontWeight: 600, color: "var(--ink)" }}>
            Anthropic API 키
          </div>
          <div style={{ marginBottom: 12, fontSize: 12.5, color: "var(--muted)" }}>
            키는 macOS 키체인에만 저장됩니다. 코드나 로그에 남지 않습니다.
          </div>
          <input
            type="password"
            autoFocus
            value={apiKey}
            onChange={(e) => { setApiKey(e.target.value); setSaved(false); setError(null); }}
            onKeyDown={handleKeyDown}
            placeholder="sk-ant-…"
            style={{
              width: "100%", boxSizing: "border-box",
              padding: "9px 12px",
              border: `1px solid ${error ? "var(--caution)" : "var(--line)"}`,
              borderRadius: "var(--r-sm)",
              background: "var(--surface-2)",
              color: "var(--ink)",
              fontSize: 13.5,
              fontFamily: "var(--mono)",
              outline: "none",
            }}
          />

          {error && (
            <div style={{
              marginTop: 8, fontSize: 12.5, color: "var(--caution)",
              padding: "6px 10px", background: "var(--caution-soft)",
              borderRadius: "var(--r-sm)",
            }}>
              {error}
            </div>
          )}

          {saved && (
            <div style={{
              marginTop: 8, fontSize: 12.5, color: "var(--primary)",
              padding: "6px 10px", background: "var(--primary-soft)",
              borderRadius: "var(--r-sm)",
            }}>
              저장됐습니다.
            </div>
          )}

          <div style={{ marginTop: 16, display: "flex", justifyContent: "flex-end", gap: 10 }}>
            <button
              onClick={onClose}
              style={{
                padding: "8px 16px", borderRadius: "var(--r-sm)",
                border: "1px solid var(--line)", background: "var(--surface)",
                cursor: "pointer", fontSize: 13, fontWeight: 600,
                fontFamily: "var(--ui)", color: "var(--ink)",
              }}
            >
              취소
            </button>
            <button
              onClick={handleSave}
              disabled={saving || !apiKey.trim()}
              style={{
                padding: "8px 18px", borderRadius: "var(--r-sm)",
                border: "none",
                background: "var(--primary)",
                color: "#fff",
                cursor: saving || !apiKey.trim() ? "not-allowed" : "pointer",
                opacity: saving || !apiKey.trim() ? 0.6 : 1,
                fontSize: 13, fontWeight: 700,
                fontFamily: "var(--ui)",
              }}
            >
              {saving ? "저장 중…" : "저장"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
