# Tabby RS 規格書

狀態：已定案，供實作與驗收使用  
基準版本：`TimLai666/tabby-rs` commit `14e2d60b9b6dee84a53c37f05eefeb803787de04`  
上游基準：Tabby `1.0.231-nightly.0` 時期的程式碼  
產品名稱：Tabby RS

## 1. 目標與痛點

Tabby 的介面、功能與可設定性符合目標使用者需求，但 Electron 會附帶完整 Chromium 與 Node.js 執行環境，增加安裝體積、記憶體占用、啟動成本及原生模組維護負擔。

Tabby RS 要以 Tauri 與 Rust 取代 Electron 主程序及所有正式版執行期 Node.js 依賴，同時保留既有 Angular、SCSS、Pug、xterm.js 介面、操作流程、設定格式及主要外掛 API。

第一個公開版本的必要結果：

1. Windows、macOS、Linux 桌面版的使用者可完成基準版 Tabby 的所有內建工作。
2. 介面版面、主題、操作流程與功能維持一致。
3. 正式版執行時不需要 Node.js。Node.js 只用於建置，以及使用者安裝或更新 npm 外掛。
4. Electron 不再存在於正式版安裝包或執行路徑。
5. 原版 Tabby 與 Tabby RS 可同時安裝、同時保留資料，使用者可以安全回退。

## 2. 使用者與主要情境

### 2.1 主要使用者

- 已使用 Tabby，偏好其介面、分頁、分割窗格、SSH、序列埠與設定能力的人。
- 希望減少 Electron 執行成本，但不願改變操作習慣的人。
- 需要在 Windows、macOS、Linux 使用同一套終端與遠端連線工作流的人。

### 2.2 核心情境

1. 使用者安裝 Tabby RS，保留原版 Tabby。
2. 首次啟動時，Tabby RS 偵測原版資料並提供單向匯入。
3. 使用者在相同介面中開啟本機 shell、分頁與分割窗格。
4. 使用者連線 SSH、SFTP、Telnet 或序列埠。
5. 使用者安裝只依賴 Angular 與 Tabby 公開 API 的第三方外掛。
6. 外掛造成啟動失敗時，Tabby RS 以安全模式啟動並讓使用者處理外掛。
7. 使用者在 Stable 與 Nightly 更新管道之間切換。
8. 使用者遇到問題時，可預覽並手動匯出已遮蔽敏感資訊的診斷包。

## 3. 功能範圍

### 3.1 桌面應用程式

使用者可以：

- 在 Tauri 視窗中使用完整 Tabby UI。
- 使用原版的標題列、視窗控制、透明度、模糊效果、深色模式與平台外觀設定；系統 WebView 造成的細微字型、捲軸或渲染差異可以接受。
- 使用全域快捷鍵顯示或隱藏視窗。
- 使用 Quake／Docking 模式。
- 使用單一實例、系統選單、Dock／工作列進度、通知、剪貼簿、檔案選擇、拖放、外部網址開啟等桌面整合。
- 在原版 Tabby 仍安裝時正常使用 Tabby RS，不互相覆寫程式、資料或更新管道。

### 3.2 本機終端

使用者可以：

- 建立、編輯及啟動自訂 shell profile。
- 使用基準版支援的 PowerShell、PowerShell Core、CMD、WSL、Git Bash、Cygwin、MSYS2、Cmder、Visual Studio Developer Shell、macOS 預設 shell 與 Linux／POSIX shell。
- 使用 PTY 調整尺寸、輸入、輸出、訊號、程序樹、目前工作目錄、工作階段還原及輸出背壓。
- 在 Windows 使用 Clink 體驗及 UAC 提權工作階段。
- 使用既有自動 sudo 密碼功能。

### 3.3 終端介面與互動

使用者可以：

- 使用 xterm.js 提供的 Unicode、雙寬字元、連字、字型 fallback、滑鼠、選取與終端控制序列能力。
- 使用分頁、任意巢狀分割窗格、拖放、重新命名、排序、關閉與啟動時工作階段復原。
- 使用複製、貼上、智慧 Ctrl-C、右鍵貼上、選取即複製、多行貼上警告、括號貼上、搜尋、鈴聲及內容選單。
- 看到程序進度、工作列進度及程序完成通知。
- 點擊 URL、IP 與檔案路徑，並在開啟應用程式 URI 前看到安全確認。
- 使用 Zmodem 傳輸及終端輸出匯出。

第一版保留 xterm.js，但其他模組不得再直接依賴具體 renderer。終端 renderer 必須透過抽象介面接入，讓未來可以替換為 Rust renderer；第一版不實作 Rust renderer。

### 3.4 SSH 與 SFTP

使用者可以：

- 建立、編輯、複製、匯入及刪除 SSH profiles。
- 使用密碼、私鑰、私鑰密語、SSH agent、Pageant 與 Windows OpenSSH Agent 等基準版驗證方式。
- 驗證 host key，查看並處理 host key 變更。
- 使用 jump host、自動 jump host、登入腳本、連線重試與工作階段復原。
- 使用本機、遠端及動態 port forwarding。
- 使用 X11 forwarding 與 agent forwarding。
- 使用 SFTP 瀏覽、上傳、下載、重新命名、刪除、建立資料夾及內容選單。
- 從 OpenSSH 設定及基準版既有來源匯入連線資料。

### 3.5 Telnet 與序列埠

使用者可以：

- 建立及使用 Telnet profiles。
- 建立及使用序列埠 profiles。
- 在序列埠使用自動重新連線、readline、十六進位逐位元組輸入、hexdump 輸出及換行轉換。

### 3.6 設定、主題與快捷鍵

使用者可以：

- 使用既有設定頁與所有基準版設定項目。
- 使用原本的主題、終端配色、字型、profile 編輯器與快捷鍵編輯器。
- 使用多段快捷鍵及基準版預設快捷鍵。
- 修改設定後即時看到適用的效果。

### 3.7 外掛

第一版外掛相容範圍：

- 外掛使用 Angular 與 Tabby 公開 API。
- 外掛以 npm 套件發布，package 關鍵字包含 `tabby-plugin`、`tabby-builtin-plugin` 或既有 legacy 關鍵字。
- 外掛不直接依賴 Electron、Node.js 執行期 API、Node 原生模組或 Tabby 私有內部 API。

使用者可以：

- 在外掛管理器搜尋 npm 上所有符合 Tabby 關鍵字的外掛。
- 在系統已安裝 Node.js 與 npm 時安裝、更新及移除外掛。
- 在沒有 Node.js 時正常使用主程式；只有外掛安裝及更新功能停用，介面顯示安裝指引。
- 使用沿用 Tabby 的高信任外掛模型。Tabby RS 不新增逐項權限提示。
- 在外掛造成啟動失敗時進入安全模式。安全模式只載入內建外掛，並提供重試、停用及移除問題外掛的操作。

第一版不發布 Tabby RS 專屬外掛 API。既有 `tabby-core`、`tabby-local`、`tabby-settings`、`tabby-terminal` 等匯入名稱由應用程式內建相容映射提供，不冒用或重新發布上游 npm 套件。

### 3.8 設定與資料遷移

- Tabby RS 使用獨立程式 ID、資料目錄、更新管道與憑證 namespace。
- 相容設定繼續使用既有 `config.yaml` 結構。
- Tabby RS 專屬狀態存入 `tabby-rs.json`，不得加入 `config.yaml`。
- 首次啟動時，使用者可以單向匯入原版 Tabby 的設定、profiles、外掛清單及秘密。
- 匯入後不再與原版雙向同步。
- 匯入不能修改或刪除原版資料。
- 秘密來源可能是 Tabby Vault 或作業系統鑰匙圈。需要解鎖或系統授權時，介面必須明確要求使用者操作。
- 無法讀取的秘密保留 profile，但標記為需重新輸入。
- 匯入完成後顯示成功、略過及失敗項目。

`tabby-rs.json` 至少包含：

```json
{
  "schemaVersion": 1,
  "firstRunImport": {
    "completed": false,
    "source": null,
    "completedAt": null,
    "reportPath": null
  },
  "updateChannel": "stable",
  "lastStableBackup": null,
  "safeMode": {
    "active": false,
    "suspectedPlugins": []
  },
  "diagnostics": {
    "localLogging": true
  }
}
```

### 3.9 Vault 與秘密儲存

- 第一版以 Rust 重作 Tabby Vault 行為。
- 第一版完整讀寫 Vault v1，不自動升級格式。
- Vault v1 必須相容既有 PBKDF2-SHA512、AES-256-CBC、salt、IV、版本欄位及 JSON 內容格式。
- Vault 可保存設定、SSH 密碼、私鑰密語及檔案。
- 未啟用 Vault 時，使用作業系統鑰匙圈。
- 程式內預留未來 Vault v2 的格式分派點，但第一版不提供 v2。

### 3.10 更新

- 啟動後自動檢查更新。
- 發現新版本時先顯示版本與變更，使用者確認後才下載及安裝。
- 同一個程式可以在設定中切換 Stable 或 Nightly。
- Stable 與 Nightly 共用資料目錄。
- 切換管道及安裝更新前自動備份設定與內部狀態。
- Stable 無法讀取 Nightly 產生的新版格式時，自動還原最後一份 Stable 備份，並保留無法讀取的資料供診斷。
- 更新包使用 Tauri updater 簽章驗證。

### 3.11 診斷與隱私

- 完全移除 Sentry、Mixpanel 與其他自動遙測。
- 不自動上傳錯誤、使用行為、裝置資訊或診斷資料。
- 在本機保存可輪替紀錄。
- 使用者可以手動產生診斷包。
- 診斷包包含程式紀錄、版本、作業系統摘要、設定摘要、外掛清單及崩潰資訊。
- 診斷包必須遮蔽密碼、Token、私鑰、私鑰密語、主機名稱、IP、使用者名稱及可識別路徑。
- 匯出前顯示檔案清單及預覽，使用者確認後才寫出。

### 3.12 發布

第一個公開版本必須先完成全部基準功能，不發布缺少核心功能的公開預覽版。

發布格式：

- Windows：NSIS
- macOS：DMG
- Linux：AppImage、DEB、RPM

Tabby RS 永遠不做 Windows Authenticode 或 macOS Developer ID 系統程式碼簽章。文件必須說明 Windows SmartScreen、macOS Gatekeeper 與企業政策可能顯示警告或阻擋安裝。Tauri 更新包仍必須使用自己的 updater 簽章。

正式版本號：

- Stable：`1.0.231-tabbyrs.N`
- Nightly：`1.0.231-tabbyrs.N.nightly.YYYYMMDD.K`
- 只有採用新的上游基準時，才更新前段上游版本。

## 4. 明確不做與延後項目

第一版不做：

- 不以 GPUI 重寫 UI。
- 不以 Rust 重寫終端 renderer。
- 不重新設計 Tabby 介面。
- 不新增 Tabby RS 專屬外掛 API。
- 不保證直接依賴 Electron、Node 執行期或原生模組的第三方外掛相容。
- 不替 `tabby-web` 新增功能。
- 不內附 Node.js。
- 不設定獨立於 Tauri 的最低作業系統版本政策。
- 不設定硬性安裝體積、RAM 或啟動速度門檻。
- 不做自動遙測。
- 不做 Windows 或 macOS 系統程式碼簽章。
- 不實作 Vault v2。

延後：

- Vault 驗證式加密格式。
- Rust 原生終端 renderer。
- Tabby RS 專屬外掛能力。
- 正式獨立品牌圖示。開發階段先沿用原圖示並加上 `RS` 標記；公開發布前必須重新檢查品牌風險。

## 5. 技術與環境限制

- 正式版 runtime 僅包含 Tauri、Rust、系統 WebView 與必要原生資源。
- Angular、SCSS、Pug、TypeScript 與 xterm.js 保留。
- Node.js 只可出現在建置流程及使用者主動執行的 npm 外掛安裝／更新流程。
- 遷移期間 Electron 與 Tauri 可以並存，並共用同一套前端；全部功能切換及通過測試後才能移除 Electron。
- `tabby-web` 必須持續可建置、可使用；桌面專屬實作不得破壞 web providers。
- 支援範圍跟隨 Tauri 及所選必要依賴能運作的平台。若某個必要功能額外縮小範圍，必須在該功能 issue 明確記錄。
- 上游同步策略為鎖定目前基準，只挑選安全修正與必要 bug fix，不完整合併上游新功能。

## 6. 主要流程與例外行為

### 6.1 首次啟動

1. Tabby RS 建立自己的資料目錄。
2. 偵測原版 Tabby 資料。
3. 若不存在原版資料，直接建立預設設定。
4. 若存在原版資料，顯示可匯入項目。
5. 使用者確認後複製非敏感設定。
6. 需要 Vault 主密碼或系統鑰匙圈授權時逐步提示。
7. 完成後顯示報告，保留原版資料不變。
8. 匯入中斷時，下次啟動可以重試，不得留下半份已宣告完成的狀態。

### 6.2 外掛啟動失敗

1. 記錄最後正在載入的外掛與錯誤。
2. 本次改為只載入內建外掛。
3. 顯示安全模式處理畫面。
4. 使用者可重試、停用或移除疑似外掛。
5. 未經使用者操作，不永久刪除外掛。

### 6.3 Nightly 切回 Stable

1. 切換前建立備份。
2. Stable 啟動時驗證 `config.yaml` 與 `tabby-rs.json` schema。
3. 可讀時直接使用。
4. 不可讀時還原最後一份 Stable 備份。
5. 保留不相容版本資料並顯示通知，不靜默刪除。

### 6.4 Node.js 不存在

- Tabby RS 正常啟動。
- 已安裝且不需要 Node runtime 的相容外掛仍可載入。
- 安裝及更新按鈕停用並顯示 Node.js／npm 檢查結果與安裝指引。

## 7. 成功指標與驗收

第一版完成條件：

1. 基準版所有內建 user-facing 功能都有對應 parity 測試結果。
2. Windows、macOS、Linux CI 均能建置正式安裝包。
3. Windows、macOS、Linux 至少各完成一次完整人工驗收矩陣。
4. 正式安裝包不包含 Electron 或 Node.js runtime。
5. 原版 Tabby 與 Tabby RS 可以並存。
6. 首次匯入不修改原版資料。
7. `config.yaml` 相容測試通過。
8. Vault v1 固定測試向量可雙向讀寫。
9. `tabby-web` 建置與核心流程測試通過。
10. Sentry、Mixpanel 與自動遙測端點不存在。
11. 安裝體積、閒置 RSS、冷啟動時間及大量輸出吞吐均產生可重現報告。這些數值不設硬性發布門檻，但明顯退化必須說明並修正或記錄接受理由。

## 8. 關鍵假設與風險

### 高風險

- 第三方外掛目前依賴 CommonJS、Node module resolution 或 Electron 的比例可能高於預期。第一版只承諾公開 Angular／Tabby API 相容。
- 不做系統程式碼簽章會造成 SmartScreen、Gatekeeper 或企業政策阻擋。
- 第一版必須完整追平才公開發布，會拉長首次發布時間。
- 高信任外掛模型允許惡意外掛使用 Tabby RS 對外公開的高權限 API。

### 中風險

- 系統 WebView 版本差異可能造成字型、透明度、拖放、IME 或 CSS 行為差異。
- Vault v1 使用 AES-CBC 且沒有完整性驗證；第一版為相容性接受此限制。
- Linux WebKitGTK 與各套件格式的依賴差異可能造成發行版間行為不一致。
- SSH X11、agent forwarding、Pageant、Windows OpenSSH Agent 與跳板連線跨平台差異大。

### 低風險

- Angular UI 與 xterm.js 可以保留，主要變更集中在 host provider 與系統能力。
- MIT 授權允許 fork 與修改，但必須保留授權及著作權聲明。

## 9. AI agent 判斷邊界

實作者可以自行決定：

- Rust crate 與內部模組切法。
- Tauri command／event 的內部命名與序列化細節。
- 測試框架、mock 實作與 CI job 切分。
- 在不改變使用者行為的前提下重構 TypeScript providers。
- 遷移期間每個功能先接 Electron 或 Tauri backend 的 feature flag 細節。

實作者必須回來確認：

- 刪除、降級或改變任何基準版 user-facing 功能。
- 改變 `config.yaml`、Vault v1 或既有外掛公開 API。
- 新增外部服務、帳號、付費憑證或資料上傳。
- 改變產品名稱、正式圖示或 app identity。
- 改變外掛權限模型。
- 改變第一版「完整追平後才公開發布」的門檻。

## 10. 決策紀錄

- 使用 Tauri 完整取代 Electron，保留 Angular／SCSS／xterm.js。
- 鎖定目前上游基準，只 cherry-pick 安全與必要修正。
- 第一版同步涵蓋 Windows、macOS、Linux。
- 完整追平後才公開發布。
- 保留使用 Tabby 公開 API 的純 Angular 第三方外掛。
- UI 行為與版面一致，允許系統 WebView 細微渲染差異。
- 首次單向匯入，之後使用獨立資料目錄。
- 秘密經使用者授權從原 Vault 或系統鑰匙圈匯入。
- 正式版 runtime 不含 Node.js。
- 遷移期間 Electron 與 Tauri 並存。
- Tabby RS 與原版可同時安裝。
- 預設 CLI／scheme 為 `tabby-rs`／`tabby-rs://`；無衝突時可啟用 `tabby` 相容別名。
- 第一版保留 xterm.js，新增 renderer 抽象。
- 外掛沿用高信任模型。
- 外掛仍從 npm 搜尋與安裝，不維護相容清單。
- 外掛失敗時進入安全模式。
- 桌面版發布，`tabby-web` 維持可建置與可使用。
- 不設硬性效能門檻。
- 作業系統支援跟隨 Tauri。
- 使用者自行安裝 Node.js／npm 以管理外掛；缺少時主程式仍可使用。
- 自動檢查更新，使用者確認後安裝。
- 提供 Stable 與 Nightly，同一程式內切換並共用資料。
- 切換或更新前備份，必要時還原 Stable 備份。
- 正式名稱為 Tabby RS。
- 第一版不公開 Tabby RS 專屬外掛 API。
- 開發階段沿用原圖示並加 RS 標記。
- 完全移除遙測。
- 保存本機輪替紀錄並手動匯出已遮蔽診斷包。
- 永遠不做 Windows／macOS 系統程式碼簽章。
- 發布 NSIS、DMG、AppImage、DEB、RPM。
- 版本使用 `上游版本-tabbyrs.N`。
- 每個使用者可感知功能一張 issue；共用基礎集中在前置 issue。
- Issue 提供完整介面、資料結構、函式簽章、關鍵實作及測試程式碼。
- 保留 `config.yaml`，Tabby RS 狀態使用 `tabby-rs.json`。
- Rust 重作 Vault，第一版完整沿用 Vault v1。

## 11. Issue 地圖

1. Epic：Tauri 遷移與完整功能追平
2. 建立 Tauri 桌面殼層與 Rust／Angular host bridge
3. 視窗、Quake／Docking、全域快捷鍵與桌面整合
4. Side-by-side identity、CLI、URL scheme 與 Windows portable mode
5. 設定儲存、首次匯入、內部狀態與備份
6. Vault v1、系統鑰匙圈與秘密匯入
7. 本機 shell profile 偵測與環境建立
8. PTY 工作階段、輸出背壓、程序樹、CWD 與還原
9. Windows shell、Clink、UAC 與自動 sudo 密碼
10. xterm.js renderer 抽象與渲染相容
11. 終端輸入、剪貼簿、滑鼠、貼上警告與內容選單
12. 分頁、巢狀分割窗格、拖放與工作階段復原
13. 程序進度、完成通知、搜尋、Linkifier 與 URI 安全確認
14. Zmodem、終端輸出匯出與一般檔案傳輸
15. SSH profiles、匯入、驗證與 host key
16. SSH jump host、port forwarding、X11、agent forwarding 與登入腳本
17. SFTP 瀏覽與傳輸
18. Telnet profiles 與工作階段
19. 序列埠 profiles、模式與重新連線
20. 設定頁、主題、配色、字型、快捷鍵與 profile 編輯器
21. npm 外掛搜尋、安裝、更新、移除與 Node.js 偵測
22. 外掛 runtime 相容層、模組映射與安全模式
23. Stable／Nightly updater、簽章、備份與回退
24. 本機紀錄、診斷包、敏感資料遮蔽與移除遙測
25. Windows／macOS／Linux 安裝包與 CI 發布
26. 維持 `tabby-web` 建置與共用前端相容
27. 完整 parity 驗收、效能量測、授權與發布文件
