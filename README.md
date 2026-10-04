<p align="center">
  <img src="docs/brand/versora.png" width="112" height="112" alt="Versora icon">
</p>

<h1 align="center">Versora</h1>

<p align="center">
Translate files on your own computer. Drop one file, a folder, or a zip — you get a matching translated file for each input, in the same folder shape.
</p>

<p align="center">Previously called Smart File Translation System. Existing links still work.</p>

## Related apps

<table>
  <tr>
    <td width="50%" align="center" valign="top">
      <a href="https://github.com/mimateinn/Versora"><img src="docs/brand/versora.png" width="64" height="64" alt="Versora icon"></a>
      <h3><a href="https://github.com/mimateinn/Versora">Versora</a></h3>
      <p>Translate files on your own computer, keeping the original folder structure.</p>
    </td>
    <td width="50%" align="center" valign="top">
      <a href="https://github.com/mimateinn/Litora"><img src="docs/brand/litora.png" width="64" height="64" alt="Litora icon"></a>
      <h3><a href="https://github.com/mimateinn/Litora">Litora</a></h3>
      <p>A desktop library and reader for your books.</p>
    </td>
  </tr>
</table>

## What it does

It reads txt, md, docx, pdf, json, csv, tsv, yaml, po, xliff, xlsx, html, srt, and vtt. Game-text mode changes only the words players see. A glossary keeps important terms consistent later.

The screen comes in 12 languages, light or dark. You pick the model. Folder and zip jobs can run 1–8 files at a time (default 2). Finished files go to `data/outputs/`, and you can download them in the browser.

It uses official developer APIs (OpenAI, Anthropic, Gemini, xAI, and others). If the official Grok CLI or Codex CLI is already installed and signed in on this computer, you can use those too. Chat websites are not supported.

You can check for official updates in the app, or just start it — at most once a day.

## How to use

1. Download this folder from GitHub (green **Code** button → **Download ZIP**) and unzip it.
2. On Windows, double-click `start.bat`. On Mac or Linux, run `./start.sh` from this folder.
3. Wait for the browser. The first start can take a few minutes. The starter installs what it needs and opens the app. An existing `.env` is left alone.
4. If it asks for a key, put the key in `.env` in this folder, save, and start again.

## Keys stay here

Keys live only in a local `.env`. The repository has no secrets. For extra options later, add them in that same file.

## Updates

### v0.1.1

- One-click start: `start.bat` (Windows) and `start.sh` (Mac / Linux).
- Folder and zip jobs write one output per input file, same folder shape.
- More types: json, csv, tsv, yaml, po, xliff, xlsx, html, srt, vtt.
- Game-text mode, plus a glossary for consistent terms.
- Official developer APIs only. Optional official Grok CLI / Codex CLI if already installed and signed in. No chat websites.
- Pick a model. Folder and zip jobs can run 1–8 files at a time (default 2).
- Official-package updates from the app or on start (at most once a day).
- Settings: Translate / Settings tabs, Appearance / Translation / Keys / Glossary, light and dark.

### v0.1.0

- First public version.

---

# Versora

在你自己的電腦上翻譯檔案。丟一個檔案、整個資料夾，或一個 zip——每個輸入檔都會得到對應的譯文，資料夾形狀相同。

Versora 前稱「智能檔案翻譯系統」。舊連結仍然有效。

## 能做什麼

支援 txt、md、docx、pdf、json、csv、tsv、yaml、po、xliff、xlsx、html、srt、vtt。遊戲文字模式只改玩家會看到的字。用語表讓重要用詞之後保持一致。

畫面有 12 種語言，淺色或深色。可選模型。資料夾／zip 一次可跑 1–8 個檔（預設 2）。譯文在 `data/outputs/`，也可以在瀏覽器下載。

用官方開發者 API（OpenAI、Anthropic、Gemini、xAI 等）。如果這台電腦已安裝並已登入官方 Grok CLI 或 Codex CLI，也可以用。不支援聊天網站。

可在程式裡檢查官方更新，或直接啟動——一天最多查一次。

## 怎麼用

1. 從 GitHub 下載這個資料夾（綠色 **Code** 按鈕 → **Download ZIP**），解壓縮。
2. Windows：連按兩下 `start.bat`。Mac 或 Linux：在這個資料夾執行 `./start.sh`。
3. 等瀏覽器打開。第一次可能要幾分鐘。啟動檔會裝好需要的東西並打開程式。已有的 `.env` 不會被覆蓋。
4. 如果要你填金鑰，把金鑰寫進這個資料夾的 `.env`，存檔後再啟動一次。

## 金鑰留在這台電腦

金鑰只放本機 `.env`，倉庫不含密鑰。之後要改更多選項，寫在同一個檔就好。
