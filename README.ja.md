# RunScope

[GitHubトップ README](README.md) / [English](README.md#runscope-english)

Windows向けの軽量RAM/VRAMプロセスインスペクターです。

RunScopeは、実行中プロセスのRAM、NVIDIA VRAM、ローカルWeb UI候補を手動で確認するための小さなネイティブデスクトップツールです。AI、Python、ComfyUI、Forge、Ollama、Node、VS Code、ターミナル、WSL、Codex/Claude系ツールを使った作業後のプロセス整理を想定しています。

このアプリはデフォルトで手動ロード方式です。起動直後にプロセス収集せず、UIフレームごとの監視もしません。MVPではCPU使用率も取得しません。最新の状態を見たいときだけ `Load / Reload` を押します。

## スクリーンショット

![RunScope GUI](docs/images/runscope-main.png)

スクリーンショットは `Load / Reload` 後に実際のプロセス一覧を表示している画面例です。

## 目的

ローカルAIや開発ツールは、Pythonサーバー、WebUI、ターミナル配下の子プロセス、Node開発サーバー、モデル実行プロセス、GPUジョブなどを残すことがあります。Windowsのタスクマネージャーでも一部は確認できますが、次のような判断には最適化されていません。

- どのプロセスがRAMやVRAMを使っているか
- そのプロセスがターミナル、VS Code、WSL、Codex、Claude、Python、WebUI系のツリーに属しているか
- 終了前に、そのプロセスがlocalhostで何かを開いていないか
- 対象PID一覧を確認してから、単体またはプロセスツリーごと終了できるか

RunScopeは、このプロセス整理ワークフローに絞っています。

## かんたんダウンロード

Rust環境がない場合は、GitHub Releasesからzipをダウンロードしてください。

[Latest Release](https://github.com/AiWithYou/RunScope/releases/latest)

1. `RunScope-windows-x64.zip` をダウンロードします。
2. zipを展開します。
3. `RunScope.exe` を起動します。
4. 起動後、`Load / Reload` を押してプロセス一覧を読み込みます。

`SHA256SUMS.txt` でダウンロードしたexe/zipのSHA256を確認できます。

Windowsの警告が出る場合があります。これは個人ビルドの未署名exeでよく出る警告です。入手元がこのリポジトリのReleaseであることを確認してから実行してください。

## 主な機能

- `Load / Reload` による手動スナップショット取得
- 任意のAuto refresh、デフォルトOFF
- 前回スナップショットとの差分（New / Changed / Exited、RAM / VRAM増減）
- RAM表示、既知VRAM表示、表示中プロセスの合計値
- `http://127.0.0.1:7860` のようなローカルTCP待ち受けURL候補の表示
- テーブル上の `Local Web` クリックでブラウザ起動、右クリックでURLコピー
- Compact / Advanced テーブル表示切り替え
- 1000件以上でも軽くスクロールできる表示範囲のみのテーブル描画
- プロセス一覧と下部詳細パネルの高さをドラッグで変更
- 親PIDと親プロセス名の表示
- プロセス年齢、実行ファイルパス、コマンドライン、CWD、仮想メモリの詳細表示
- RAM / VRAM / 増加量 / 名前 / PID / 年齢の12種類のソートとクリック可能な列見出し
- 通常検索に加え、`name:`、`port:`、`ram:>`、除外語などの構造化Search
- ANDで組み合わせられるQuick filters: `Python`、`GPU Active`、`Local Web`、`Codex/Claude`、`Heavy RAM`、`Heavy VRAM`、`New / Changed`
- 保護対象プロセスの非表示
- 表示中テーブルのTSV、PID一覧、選択プロセスJSON、診断情報のクリップボードコピー
- `Close`、`Kill`、`Kill Tree`
- 行右クリックメニューから `Open Local Web`、PID/名前/Path/Command Line/CWDコピー、終了操作
- 詳細パネルからEXE/CWDを開く、親・子プロセスへ移動、summary/JSONコピー
- `Close` / `Kill` / `Kill Tree` 前の対象PID、RAM/VRAM合計、Local Web数の確認
- 編集可能な追加保護リストに加え、解除不能なWindows重要プロセス保護
- ライト/ダークテーマに追従する選択表示
- collector別所要時間と詳細な `--self-check`
- `Settings` 画面から設定編集、リセット、`settings.json` を開く/再読み込み。閉じると未保存変更を自動保存

## ソースからビルド

WindowsにRustをインストールしてから実行します。

```powershell
cargo build --release
```

配布しやすい実行ファイルを作る場合:

```powershell
.\build_release.ps1
```

次のファイルが作成されます。

```text
dist\RunScope.exe
dist\RunScope-windows-x64.zip
dist\SHA256SUMS.txt
```

GitHub ActionsでもWindows上で `dist\RunScope.exe`、`dist\RunScope-windows-x64.zip`、`dist\SHA256SUMS.txt` をビルドし、`RunScope-windows-x64` artifactとしてアップロードします。`v*` タグをpushすると同じ成果物をGitHub Releaseへ添付します。

## 起動

```powershell
.\run_windows.bat
```

`run_windows.bat` は次の順に起動対象を探します。

1. `dist\RunScope.exe`
2. `target\release\runscope.exe`
3. `cargo run --release`

明示的に起動直後のスナップショット取得まで行う場合:

```powershell
.\dist\RunScope.exe --load
```

通常起動では従来どおり自動収集しません。診断は `RunScope.exe --self-check`、バージョン確認は `RunScope.exe --version` です。

通常レンダラーの画面をBMPへ保存する場合（`--load` と併用すると収集完了後に保存）:

```powershell
.\dist\RunScope.exe --load --screenshot "$env:TEMP\runscope.bmp"
```

## 使い方

1. RunScopeを起動します。
2. `Load / Reload` を押します。
3. SortからRAM、VRAM、増加量、名前、PID、年齢で並び替えます。対応する列見出しもクリックできます。
4. Quick filterを1つ以上選び、AND条件で絞り込みます。`All` は全Quick filterを解除します。
5. `Compact` では主要列だけ、`Advanced` ではParent/Path/Command Lineも含めて確認できます。
6. 行を選択して下部詳細パネルを確認します。Path、Command Line、CWD、Virtual Memoryは詳細パネルに表示されます。
7. プロセス一覧と詳細パネルの境界をドラッグすると、一覧と詳細の高さを変更できます。
8. 終了前に `Local Web` を確認します。そのPIDが開いているWebUIや開発サーバー候補を見つけられます。
9. `Local Web` 列のリンクをクリックして候補URLを開けます。複数ポートがある場合もprimary URLを表示し、詳細パネルに全URLを表示します。
10. 行を右クリックすると `Open Local Web`、各種コピー、`Close`、`Kill`、`Kill Tree` を使えます。
11. 下部詳細パネルからEXE/CWDを開く、親・子プロセスへ移動、summary/JSONをコピーできます。
12. 対象を確認してから `Close`、`Kill`、`Kill Tree` を使います。
13. 上部の `Copy` から表示中テーブルをExcel等へ貼れるTSVとしてコピーできます。
14. 詳細設定は `Settings` から変更できます。

## 表示列

Compactの列:

- `Scope`
- `PID`
- `Process Name`
- `RAM MB (delta)`
- `VRAM MB (delta)`
- `Local Web`
- `Age`

Advancedでは次の列も追加表示します。

- `Parent PID`
- `Parent Name`
- `Executable Path`
- `Command Line`

Path、Command Line、CWD、Virtual MemoryはCompactでは下部詳細パネルに表示します。MVPではCPU列はありません。

2回目以降のLoadでは、同じプロセスidentity（PID、名前、開始時刻、取得できる場合はPath）だけを比較し、RAM / VRAMの増減を括弧内に表示します。PIDが再利用された場合は別プロセスとして扱います。

## Search構文

空白区切りの語はAND条件です。`-` を先頭に付けると除外、引用符で空白を含む値を指定できます。

- `name:python port:7860`
- `ram:>1024 vram:>=4096`
- `scope:gpu state:changed`
- `path:"program files" -cmd:test`
- 使用可能なfield: `pid`、`name`、`scope`、`path`、`cmd`、`parent`、`port`、`web`、`state`、`ram`、`vram`

## キーボードショートカット

- `F5` / `Ctrl+R`: `Load / Reload`
- `Ctrl+F`: Searchへフォーカス
- `Up` / `Down`: 選択を前後へ移動
- `PageUp` / `PageDown`: 10行移動
- `Home` / `End`: 先頭/末尾へ移動
- `Enter`: 選択中プロセスのprimary Local Webを開く
- `Ctrl+C`: 選択中プロセスのsummaryをコピー
- `Ctrl+Shift+C`: 表示中テーブルをTSVでコピー
- `Delete`: 選択中プロセスのKill確認を開く。保護対象プロセスでは無効です。
- `Escape`: Searchを解除。Searchが空ならQuick filterを全解除

## VRAM取得

RunScopeは、失敗してもアプリを落とさない形で複数のVRAM取得元を試します。

1. `nvml.dll` の動的ロード
2. Windows GPU Process Memoryパフォーマンスカウンター
3. `nvidia-smi` fallback

```powershell
nvidia-smi --query-compute-apps=pid,used_gpu_memory --format=csv,noheader,nounits
```

どの方法も使えない場合でも、RAMとプロセス情報は表示します。VRAMが不明なプロセスは `N/A` と表示します。

## Local Web検出

RunScopeはWindowsのTCPテーブルから、PIDごとの待ち受けポートを取得し、ローカルで開ける候補URLとして表示します。

例:

- `127.0.0.1:3000` -> `http://127.0.0.1:3000`
- `0.0.0.0:7860` -> `http://127.0.0.1:7860`
- `[::1]:8080` -> `http://[::1]:8080`
- ポート `443` または `8443` -> `https://...`

これは軽量性を優先した検出です。RunScopeはポートへアクセスしてHTTP/WebUIかどうかを検証しません。

テーブルでは、よく使われる `7860`、`8188`、`3000`、`5000`、`8000`、`8080`、`5173`、`11434` を優先してクリック先にします。複数ポートがある場合も `http://127.0.0.1:7860 (+1)` のようにprimary URLを表示し、詳細パネルで全URLを確認できます。

## 保護対象プロセス

組み込みの重要プロセス保護には次が含まれます。この一覧は `settings.json` から解除できません。

- `System`
- `Registry`
- `Idle`
- `csrss.exe`
- `wininit.exe`
- `winlogon.exe`
- `services.exe`
- `lsass.exe`
- `smss.exe`
- `svchost.exe`
- `fontdrvhost.exe`
- `Memory Compression`
- `Secure System`

さらに、既定の追加保護名として `dwm.exe` と `explorer.exe` を設定しています。追加保護名はSettingsで編集できます。保護対象プロセスはRunScopeから終了できません。またKill時は、開いたプロセスhandleから開始時刻と実行ファイルを再確認してから、その同じhandleを終了します。

## 設定

`Settings` ボタンから簡易設定画面を開けます。General、Filters、Protection、Keywords、Aboutに分けて、Refresh mode、2/5/10/30/60秒のinterval、12種類のsort、Table view、複合filter、Heavy RAM/VRAM閾値、追加保護対象名、Python判定キーワード、Codex/Claude/Terminal root判定キーワードを編集できます。

実行ファイルと同じディレクトリに `settings.json` がある場合、RunScopeはそれを読み込みます。ない場合は組み込みのデフォルト設定を使います。設定画面から `settings.json` を開く、再読み込みする、デフォルトに戻す操作もできます。Settingsの未保存変更はウィンドウを閉じると自動保存されます。

雛形:

```text
settings.example.json
```

設定できる項目:

- refresh modeとinterval
- デフォルトフィルタ
- デフォルトソート
- Compact / Advanced table view
- Heavy RAM / Heavy VRAM閾値
- 追加保護対象プロセス名（組み込み重要プロセス保護は解除不可）
- Python判定キーワード
- Codex/Claude/Terminal root判定キーワード

## 技術スタック

- Rust
- egui / eframe
- sysinfo
- windows crate
- Dynamic NVML loading
- serde / serde_json
- anyhow

Electron、Tauri、Python GUI framework、PySide runtimeは使っていません。

## 開発

フォーマット:

```powershell
cargo fmt --all
```

チェック:

```powershell
cargo check
```

テスト:

```powershell
cargo test
```

Lint:

```powershell
cargo clippy --all-targets -- -D warnings
```

リリース実行ファイルのビルド:

```powershell
.\build_release.ps1
```

GUIを起動しない診断:

```powershell
.\dist\RunScope.exe --version
.\dist\RunScope.exe --self-check
```

## 制限

- Windows専用のデスクトップアプリです。
- NVIDIA VRAM表示は、NVML、Windows GPU counters、または `nvidia-smi` の利用可否に依存します。
- `Local Web` はTCP LISTENソケットから作る候補URLであり、HTTP endpointとして検証済みではありません。
- MVPではCPU使用率は実装していません。
- プロセス情報はスナップショット方式です。最新状態を見るには `Load / Reload` を押してください。

## ライセンス

MITです。詳細は [LICENSE](LICENSE) を参照してください。
