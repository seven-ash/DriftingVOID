# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Ash (seven-ash). Additional terms (AGPL-3.0 section 7): see NOTICE.
# Setup and uninstall messages in Korean, English and Japanese, dot-sourced by
# both. The language: -Lang, else the one chosen in the app (prefs.json), else
# Windows' display language as the app's first start reads it (English or
# Japanese, else Korean).
function Get-DvLang([string]$want) {
    if ($want -in @("ko", "en", "ja")) { return $want }
    $prefs = Join-Path $env:APPDATA "DriftingVOID\prefs.json"
    if (Test-Path $prefs) {
        try {
            $p = Get-Content $prefs -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($p.settings.lang -in @("ko", "en", "ja")) { return $p.settings.lang }
        } catch {}
    }
    # the same call as the app (i18n::system_lang): the primary language id
    try {
        Add-Type -Namespace DV -Name Lang -MemberDefinition '[DllImport("kernel32.dll")] public static extern ushort GetUserDefaultUILanguage();' -ErrorAction SilentlyContinue
        switch ([DV.Lang]::GetUserDefaultUILanguage() -band 0x3ff) { 0x09 { return "en" } 0x11 { return "ja" } default { return "ko" } }
    } catch {
        switch ((Get-UICulture).TwoLetterISOLanguageName) { "en" { return "en" } "ja" { return "ja" } default { return "ko" } }
    }
}

# key = Korean, English, Japanese; {0} marks a value
$DvMsg = @{
    noexe        = @("{0} 없음: 압축을 모두 푼 폴더에서 Setup.cmd를 실행하세요.", "{0} not found: run Setup.cmd from the fully extracted folder.", "{0} が見つかりません: すべて展開したフォルダーから Setup.cmd を実行してください。")
    title        = @("설치", "Setup", "セットアップ")
    where        = @("설치 위치  {0}", "Installing to  {0}", "インストール先  {0}")
    step1        = @("1/3  프로그램 복사", "1/3  Copying the program", "1/3  プログラムのコピー")
    running      = @("Drifting VOID가 실행 중입니다. 작업을 저장하고 닫은 뒤 계속하세요.", "Drifting VOID is running. Save your work and close it to continue.", "Drifting VOID が実行中です。作業を保存して閉じてから続けてください。")
    closedEnter  = @("닫았으면 Enter", "Press Enter once it is closed", "閉じたら Enter")
    stillRunning = @("아직 실행 중이라 설치를 멈춥니다.", "It is still running; setup stops here.", "まだ実行中のため、セットアップを中止します。")
    done         = @("완료", "Done", "完了")
    step2        = @("2/3  ffmpeg (영상·소리)", "2/3  ffmpeg (video, sound)", "2/3  ffmpeg (動画・音)")
    ffFound      = @("이미 설치된 ffmpeg를 사용합니다: {0}", "Using the ffmpeg already installed: {0}", "インストール済みの ffmpeg を使います: {0}")
    ffSkip       = @("건너뜀 (영상과 소리는 ffmpeg 설치 후 사용 가능)", "Skipped (video and sound work once ffmpeg is installed)", "スキップ (動画と音は ffmpeg のインストール後に使えます)")
    ffAbout1     = @("ffmpeg는 FFmpeg 프로젝트의 별도 프로그램(GPL)입니다.", "ffmpeg is a separate program (GPL) from the FFmpeg project.", "ffmpeg は FFmpeg プロジェクトの別のプログラム(GPL)です。")
    ffAbout2     = @("GitHub의 {0} 공개 빌드를 내려받아 설치 폴더에만 둡니다 (약 80 MB).", "Setup downloads the public build from {0} on GitHub and keeps it in the install folder only (about 80 MB).", "GitHub の {0} の公開ビルドをダウンロードし、インストール先にだけ置きます (約 80 MB)。")
    ffAsk        = @("내려받을까요?", "Download it?", "ダウンロードしますか?")
    errNoBuild   = @("릴리스에서 Windows 빌드를 찾지 못함", "no Windows build in the release", "リリースに Windows 用ビルドがありません")
    errNoSum     = @("체크섬을 찾지 못함", "no checksum found", "チェックサムが見つかりません")
    errHttp      = @("다운로드 실패: HTTP {0}", "download failed: HTTP {0}", "ダウンロード失敗: HTTP {0}")
    downloading  = @("ffmpeg 내려받는 중", "Downloading ffmpeg", "ffmpeg をダウンロード中")
    errSum       = @("체크섬 불일치 (받은 파일 {0})", "checksum mismatch (downloaded file {0})", "チェックサムが一致しません (取得したファイル {0})")
    sumOk        = @("체크섬 확인 (SHA-256)", "Checksum verified (SHA-256)", "チェックサムを確認 (SHA-256)")
    errNoExe     = @("압축 안에 bin\ffmpeg.exe 없음", "no bin\ffmpeg.exe in the zip", "zip に bin\ffmpeg.exe がありません")
    errRun       = @("설치한 ffmpeg가 실행되지 않음", "the installed ffmpeg does not run", "インストールした ffmpeg が起動しません")
    ffDone       = @("설치 완료: {0}", "Installed: {0}", "インストール完了: {0}")
    ffFail       = @("ffmpeg 설치 실패: {0}", "ffmpeg setup failed: {0}", "ffmpeg のインストールに失敗: {0}")
    ffFail2      = @("앱은 그대로 쓸 수 있고, 영상과 소리만 빠집니다. 나중에 Setup.cmd를 다시 실행하거나", "The app works without it; only video and sound are missing. Run Setup.cmd again later, or", "アプリはそのまま使え、動画と音だけが使えません。あとで Setup.cmd を再実行するか、")
    ffFail3      = @("winget install Gyan.FFmpeg 로 설치하세요.", "install it with winget install Gyan.FFmpeg.", "winget install Gyan.FFmpeg でインストールしてください。")
    step3        = @("3/3  바로 가기 · .void 파일 연결", "3/3  Shortcuts · .void file type", "3/3  ショートカット · .void ファイルの関連付け")
    shellSkip    = @("건너뜀 (-NoShell)", "Skipped (-NoShell)", "スキップ (-NoShell)")
    manualLink   = @("{0} 사용 설명서", "{0} manual", "{0} マニュアル")
    deskAsk      = @("바탕 화면에 바로 가기를 만들까요?", "Add a desktop shortcut?", "デスクトップにショートカットを作りますか?")
    fileType     = @("Drifting VOID 프로젝트", "Drifting VOID project", "Drifting VOID プロジェクト")
    shellDone    = @("시작 메뉴 · .void 연결 · 앱 목록(설정 > 앱) 등록 완료", "Start menu · .void files · listed in Settings > Apps", "スタートメニュー · .void の関連付け · アプリ一覧(設定 > アプリ)に登録完了")
    finished     = @("설치가 끝났습니다.", "Setup is complete.", "セットアップが完了しました。")
    removeHow    = @("제거: 설정 > 앱 > Drifting VOID, 또는 {0}", "To remove: Settings > Apps > Drifting VOID, or {0}", "削除: 設定 > アプリ > Drifting VOID、または {0}")
    runAsk       = @("지금 실행할까요?", "Start it now?", "今すぐ起動しますか?")
    unTitle      = @("제거", "Uninstall", "アンインストール")
    unKeep       = @(".void 프로젝트 파일은 지우지 않습니다.", ".void project files are never deleted.", ".void プロジェクトファイルは削除しません。")
    unAsk        = @("제거할까요?", "Uninstall?", "アンインストールしますか?")
    unRunning    = @("Drifting VOID가 실행 중입니다. 저장하고 닫은 뒤 다시 실행하세요.", "Drifting VOID is running. Save, close it and run this again.", "Drifting VOID が実行中です。保存して閉じてから、もう一度実行してください。")
    unData       = @("설정과 오류 기록({0})도 지울까요?", "Also delete settings and the crash log ({0})?", "設定とエラーログ({0})も削除しますか?")
    unDone       = @("제거했습니다.", "Uninstalled.", "アンインストールしました。")
    closeEnter   = @("Enter로 닫기", "Press Enter to close", "Enter で閉じる")
}

# the message in $DvLang, values filled in
function T([string]$k) {
    $i = [Array]::IndexOf(@("ko", "en", "ja"), $script:DvLang)
    if ($i -lt 0) { $i = 0 }
    $s = $DvMsg[$k][$i]
    if ($args.Count) { $s = $s -f $args }
    return $s
}
