import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { writeTextFile, readTextFile, BaseDirectory } from "@tauri-apps/plugin-fs";

interface ProgressEvent {
  current: number;
  total: number;
  current_file: string;
  status: string;
}

interface LogEvent {
  level: string;
  message: string;
}

interface CopyCompleteEvent {
  copied: number;
  skipped: number;
  errors: number;
  cancelled: boolean;
}

interface SystemStatus {
  ffprobe_installed: boolean;
  ffprobe_path: string | null;
  os_type: string;
}

function App() {
  const [sourcePath, setSourcePath] = useState<string>("");
  const [destPath, setDestPath] = useState<string>("");
  const [isRunning, setIsRunning] = useState(false);
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [result, setResult] = useState<CopyCompleteEvent | null>(null);
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [showFfprobeModal, setShowFfprobeModal] = useState(false);
  const [installStatus, setInstallStatus] = useState<string>("");
  const logContainerRef = useRef<HTMLDivElement>(null);

  const getInstallCommand = (osType: string): string => {
    switch (osType) {
      case "macos": return "brew install ffmpeg";
      case "windows": return "winget install ffmpeg";
      default: return "sudo apt install ffmpeg";
    }
  };

  const saveLogs = async (logsToSave: LogEvent[]) => {
    try {
      await writeTextFile("media_cc_logs.json", JSON.stringify(logsToSave), { baseDir: BaseDirectory.AppData });
    } catch (e) {
      console.error("Failed to save logs:", e);
    }
  };

  const loadLogs = async (): Promise<LogEvent[]> => {
    try {
      const content = await readTextFile("media_cc_logs.json", { baseDir: BaseDirectory.AppData });
      return JSON.parse(content) as LogEvent[];
    } catch {
      return [];
    }
  };

  const checkFfprobeStatus = async () => {
    const status = await invoke<SystemStatus>("check_ffprobe");
    setSystemStatus(status);
    return status;
  };

  useEffect(() => {
    checkFfprobeStatus();
    loadLogs().then((savedLogs) => {
      if (savedLogs.length > 0) {
        setLogs(savedLogs);
      }
    });
    const unlistenProgress = listen<ProgressEvent>("copy-progress", (e) => setProgress(e.payload));
    const unlistenLog = listen<LogEvent>("copy-log", (e) => setLogs((prev) => [...prev, e.payload]));
    const unlistenComplete = listen<CopyCompleteEvent>("copy-complete", (e) => {
      setResult(e.payload);
      setIsRunning(false);
      setProgress(null);
      setLogs((currentLogs) => {
        saveLogs(currentLogs);
        return currentLogs;
      });
    });
    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenLog.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (logContainerRef.current) {
      logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight;
    }
  }, [logs]);

  const selectFolder = async (type: "source" | "dest") => {
    const selected = await open({ directory: true, multiple: false, title: type === "source" ? "选择源文件夹" : "选择目标文件夹" });
    if (selected) type === "source" ? setSourcePath(selected as string) : setDestPath(selected as string);
  };

  const startCopy = async () => {
    if (!sourcePath || !destPath) return;
    setLogs([]);
    setResult(null);
    setIsRunning(true);
    try {
      await invoke("start_copy", { sourcePath, destPath });
    } catch (error) {
      setLogs((prev) => [...prev, { level: "error", message: `错误: ${error}` }]);
      setIsRunning(false);
    }
  };

  const handleInstallFfprobe = async () => {
    if (!systemStatus) return;
    const command = getInstallCommand(systemStatus.os_type);
    try {
      await writeText(command);
      setInstallStatus("copied");
      await invoke("open_terminal_with_command", { command });
      setInstallStatus("waiting");
    } catch {
      setInstallStatus("error");
    }
  };

  const handleRefreshStatus = async () => {
    setInstallStatus("checking");
    const status = await checkFfprobeStatus();
    setInstallStatus(status.ffprobe_installed ? "success" : "not_found");
    if (status.ffprobe_installed) setTimeout(() => setShowFfprobeModal(false), 1000);
  };

  const getLogColor = (level: string) => {
    switch (level) {
      case "error": return "text-red-400";
      case "warn": return "text-amber-400";
      case "success": return "text-emerald-400";
      default: return "text-slate-400";
    }
  };

  const progressPercent = progress ? Math.round((progress.current / progress.total) * 100) : 0;
  const truncatePath = (path: string, max = 35) => path.length > max ? "..." + path.slice(-max) : path;

  return (
    <div className="h-screen bg-slate-900 text-slate-100 p-3 flex flex-col text-sm select-none">
      {/* 标题栏 */}
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-1.5">
          <h1 className="text-base font-semibold text-slate-200">时光归档</h1>
          <div className="relative group">
            <span className="w-4 h-4 flex items-center justify-center text-xs text-slate-500 hover:text-slate-300 cursor-help rounded-full border border-slate-600 hover:border-slate-500">?</span>
            <div className="absolute left-0 top-6 w-56 p-2 bg-slate-800 border border-slate-700 rounded shadow-lg text-xs text-slate-300 opacity-0 invisible group-hover:opacity-100 group-hover:visible transition-all z-50">
              <p className="font-medium text-slate-200 mb-1">按拍摄日期整理媒体文件</p>
              <ul className="text-slate-400 space-y-0.5">
                <li>• 从照片/视频内部提取创建时间</li>
                <li>• 自动按 YYYY-MM-DD 归档</li>
                <li>• MD5 去重，避免重复文件</li>
              </ul>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <span className={`w-1.5 h-1.5 rounded-full ${systemStatus?.ffprobe_installed ? "bg-emerald-500" : "bg-amber-500"}`} />
          <span className="text-xs text-slate-500">
            ffprobe {systemStatus?.ffprobe_installed ? "OK" : (
              <button onClick={() => setShowFfprobeModal(true)} className="text-blue-400 hover:underline">未安装</button>
            )}
          </span>
        </div>
      </div>

      {/* 文件夹选择 */}
      <div className="space-y-2 mb-3">
        <div className="flex items-center gap-2">
          <span className="w-12 text-slate-500 text-xs">源目录</span>
          <div
            onClick={() => !isRunning && selectFolder("source")}
            className={`flex-1 bg-slate-800 border border-slate-700 rounded px-2 py-1.5 text-xs truncate cursor-pointer hover:border-slate-600 ${isRunning ? "opacity-50" : ""}`}
          >
            {sourcePath ? truncatePath(sourcePath) : <span className="text-slate-600">点击选择...</span>}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-12 text-slate-500 text-xs">目标</span>
          <div
            onClick={() => !isRunning && selectFolder("dest")}
            className={`flex-1 bg-slate-800 border border-slate-700 rounded px-2 py-1.5 text-xs truncate cursor-pointer hover:border-slate-600 ${isRunning ? "opacity-50" : ""}`}
          >
            {destPath ? truncatePath(destPath) : <span className="text-slate-600">点击选择...</span>}
          </div>
        </div>
      </div>

      {/* 操作按钮 */}
      <div className="flex gap-2 mb-3">
        <button
          onClick={startCopy}
          disabled={isRunning || !sourcePath || !destPath}
          className="flex-1 py-1.5 bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 disabled:text-slate-500 rounded text-xs font-medium transition-colors"
        >
          {isRunning ? "处理中..." : "开始整理"}
        </button>
        {isRunning && (
          <button
            onClick={() => invoke("cancel_copy")}
            className="px-3 py-1.5 bg-red-600/80 hover:bg-red-500 rounded text-xs transition-colors"
          >
            取消
          </button>
        )}
      </div>

      {/* 进度条 */}
      {(progress || result) && (
        <div className="mb-3">
          <div className="flex justify-between text-xs text-slate-500 mb-1">
            <span>{progress ? `${progress.current}/${progress.total}` : "完成"}</span>
            <span>{progress ? `${progressPercent}%` : (result?.cancelled ? "已取消" : "100%")}</span>
          </div>
          <div className="w-full bg-slate-800 rounded-full h-1.5">
            <div
              className={`h-1.5 rounded-full transition-all ${result?.cancelled ? "bg-amber-500" : result ? "bg-emerald-500" : "bg-blue-500"}`}
              style={{ width: `${progress ? progressPercent : 100}%` }}
            />
          </div>
          {progress && <p className="text-xs text-slate-600 mt-1 truncate">{progress.current_file}</p>}
        </div>
      )}

      {/* 结果统计 */}
      {result && (
        <div className="flex gap-2 mb-3 text-xs">
          <div className="flex-1 text-center py-1.5 bg-emerald-900/30 rounded">
            <span className="text-emerald-400 font-semibold">{result.copied}</span>
            <span className="text-slate-500 ml-1">复制</span>
          </div>
          <div className="flex-1 text-center py-1.5 bg-amber-900/30 rounded">
            <span className="text-amber-400 font-semibold">{result.skipped}</span>
            <span className="text-slate-500 ml-1">跳过</span>
          </div>
          <div className="flex-1 text-center py-1.5 bg-red-900/30 rounded">
            <span className="text-red-400 font-semibold">{result.errors}</span>
            <span className="text-slate-500 ml-1">错误</span>
          </div>
        </div>
      )}

      {/* 日志 */}
      <div className="flex-1 bg-slate-800/50 rounded overflow-hidden flex flex-col min-h-0">
        <div className="px-2 py-1 bg-slate-800 text-xs text-slate-500 flex justify-between">
          <span>日志</span>
          {logs.length > 0 && <button onClick={() => { setLogs([]); saveLogs([]); }} className="hover:text-slate-300">清空</button>}
        </div>
        <div ref={logContainerRef} className="flex-1 overflow-y-auto p-2 font-mono text-xs leading-relaxed">
          {logs.length === 0 ? (
            <p className="text-slate-600">等待操作...</p>
          ) : (
            logs.map((log, i) => (
              <div key={i} className={`${getLogColor(log.level)} break-all`}>{log.message}</div>
            ))
          )}
        </div>
      </div>

      {/* ffprobe 安装弹窗 */}
      {showFfprobeModal && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center p-4 z-50">
          <div className="bg-slate-800 rounded-lg p-4 w-full max-w-sm shadow-xl">
            <h3 className="font-medium mb-2">安装 ffprobe</h3>
            <p className="text-xs text-slate-400 mb-3">视频元数据提取需要 ffprobe。点击下方按钮将打开终端并自动输入安装命令。</p>
            <code className="block bg-slate-900 px-2 py-1.5 rounded text-xs text-emerald-400 mb-3">
              {systemStatus ? getInstallCommand(systemStatus.os_type) : ""}
            </code>
            <div className="flex gap-2">
              <button
                onClick={handleInstallFfprobe}
                disabled={installStatus === "waiting" || installStatus === "checking"}
                className="flex-1 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 rounded text-xs transition-colors"
              >
                {installStatus === "copied" ? "已复制..." : installStatus === "waiting" ? "请在终端完成安装" : installStatus === "success" ? "已安装!" : "一键安装"}
              </button>
              {(installStatus === "waiting" || installStatus === "checking") && (
                <button onClick={handleRefreshStatus} disabled={installStatus === "checking"} className="px-3 py-1.5 bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 rounded text-xs">
                  {installStatus === "checking" ? "..." : "检测"}
                </button>
              )}
            </div>
            {installStatus === "success" && <p className="text-xs text-emerald-400 mt-2">ffprobe 已安装成功!</p>}
            {installStatus === "not_found" && <p className="text-xs text-amber-400 mt-2">未检测到，请确保安装完成</p>}
            <button onClick={() => { setShowFfprobeModal(false); setInstallStatus(""); }} className="w-full mt-3 py-1 text-xs text-slate-500 hover:text-slate-300">
              关闭
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
