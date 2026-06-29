export interface FileNode {
  name: string;
  path: string;
  is_dir: boolean;
  size?: number;
  ext?: string;
  children: FileNode[];
}

export interface ExecResult {
  moved: number;
  staged: number;
  renamed: number;
}
