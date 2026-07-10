# RunScope Requirements

## Purpose

RunScope is a lightweight Windows RAM/VRAM process inspector. It helps find and terminate unnecessary processes or process trees started by terminals, VS Code, WSL, Codex/Claude tools, Python, ComfyUI, Forge, Ollama, Node, and general desktop apps.

## Non-Goals

- Do not recreate Task Manager.
- Do not implement CPU usage in the MVP.
- Do not poll process data every UI frame.
- Do not default to realtime monitoring.
- Do not use Electron, Tauri, Python GUI frameworks, or PySide.

## MVP Requirements

- Startup must not collect processes.
- `Load / Reload` collects one snapshot.
- Auto refresh is optional and default OFF.
- Load must run off the UI thread.
- Process columns:
  - Scope
  - PID
  - Process Name
  - RAM MB
  - VRAM MB
  - Local Web
  - Parent PID
  - Parent Name
  - Age
  - Executable Path
  - Command Line
- No CPU column.
- Sort presets:
  - RAM descending
  - VRAM descending
- Default sort is VRAM descending.
- Unknown VRAM values sort below known VRAM values.
- Filters:
  - Search text
  - Python only
  - GPU/VRAM active only
  - Codex/Claude related only
  - Hide system/protected processes
- Actions:
  - Close
  - Kill
  - Kill Tree
- Close, Kill, and Kill Tree must show a confirmation dialog with the target PID list.
- Termination confirmation should show local web/listener URLs when available.
- Protected process names must not be terminated.

## Python Detection

RunScope treats a process as Python-related when process name, executable path, or command line contains Python-focused keywords such as:

- python.exe
- pythonw.exe
- py.exe
- .py when the executable itself is Python-like
- uvicorn
- streamlit
- gradio
- jupyter
- ipython
- conda
- comfyui
- forge
- stable-diffusion-webui
- launch.py

## Codex / Terminal / Claude Detection

The following names or command-line fragments are treated as root candidates:

- codex
- openai
- claude
- wt.exe
- WindowsTerminal.exe
- cmd.exe
- powershell.exe
- pwsh.exe
- Code.exe
- wsl.exe

Descendants of root candidates are marked as Codex/Claude/Terminal-related.

## VRAM

Preferred implementation:

- Dynamically load `nvml.dll`.
- Collect compute and graphics running process memory.
- Merge memory per PID.

Fallbacks:

- Windows `GPU Process Memory(*)\Dedicated Usage`
- NVIDIA SMI:

```bat
nvidia-smi --query-compute-apps=pid,used_gpu_memory --format=csv,noheader,nounits
```

If both NVML and `nvidia-smi` are unavailable, the app must still show RAM data and mark VRAM as unavailable.
