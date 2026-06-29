import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FileNode, ExecResult } from "./types";
import { TreeNode } from "./components/TreeNode";
import { Topbar } from "./components/Topbar";
import { DogMascot } from "./components/DogMascot";
import { PlanReview, PlanSummary } from "./components/PlanReview";

export default function App() {
  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [tree, setTree] = useState<FileNode | null>(null);
  const [scanning, setScanning] = useState(false);
  const [pendingPlan, setPendingPlan] = useState<PlanSummary | null>(null);
  const [lastResult, setLastResult] = useState<ExecResult | null>(null);

  async function selectAndScan() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setFolderPath(selected);
    setScanning(true);
    setLastResult(null);
    try {
      const result = await invoke<FileNode>("scan_directory", {
        root: selected,
        maxDepth: 10,
      });
      setTree(result);
    } catch (err) {
      console.error("scan_directory failed:", err);
    } finally {
      setScanning(false);
    }
  }

  function handleExecuted(result: ExecResult) {
    setPendingPlan(null);
    setLastResult(result);
    // 트리 갱신
    if (folderPath) {
      invoke<FileNode>("scan_directory", { root: folderPath, maxDepth: 10 })
        .then(setTree)
        .catch(console.error);
    }
  }

  return (
    <>
      <Topbar folderPath={folderPath} onFolderSelect={selectAndScan} />
      <div className="shell">
        <aside className="sidebar">
          <div className="side-head">폴더 트리</div>
          <div className="tree">
            {scanning && <div className="scan-hint">스캔 중…</div>}
            {!scanning && !tree && (
              <div className="scan-hint">폴더를 선택하면 트리를 표시합니다.</div>
            )}
            {!scanning && tree && <TreeNode node={tree} depth={0} />}
          </div>
          <div className="side-foot">
            <button className="add-btn" onClick={selectAndScan}>
              ＋ 폴더 추가
            </button>
          </div>
        </aside>

        <main className="main">
          <div className="chat">
            {!folderPath && (
              <div className="scan-hint" style={{ marginTop: "40px" }}>
                왼쪽에서 폴더를 선택하면 TidyDog이 분석을 시작합니다.
              </div>
            )}
            {lastResult && (
              <div style={{
                background: "var(--primary-soft)",
                border: "1px solid var(--primary)",
                borderRadius: "var(--r-sm)",
                padding: "14px 18px",
                fontSize: 14,
                color: "var(--primary)",
                fontWeight: 600,
              }}>
                완료 — 이동 {lastResult.moved}개 · 격리 {lastResult.staged}개 · 리네임 {lastResult.renamed}개
              </div>
            )}
          </div>
          <div className="composer">
            <div className="input-bar">
              <input
                placeholder="무엇을 정리할까요? (Phase 3에서 활성화됩니다)"
                disabled
              />
              <button className="send-btn" disabled>↑</button>
            </div>
          </div>
          <DogMascot state={scanning ? "생각 중" : pendingPlan ? "검토 중" : "대기 중"} />
        </main>
      </div>

      {pendingPlan && folderPath && (
        <PlanReview
          plan={pendingPlan}
          root={folderPath}
          onExecuted={handleExecuted}
          onCancel={() => setPendingPlan(null)}
        />
      )}
    </>
  );
}
