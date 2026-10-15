# ChatGPT presentation assistant

**Status: 2026-10-15 — desktop integration with browser sign-in, presentation tools, visual inspection, search and context references.**

DeckCraft's ChatGPT sidebar uses your ChatGPT account and the public OpenAI Responses API.
It is implemented in Rust and does not require Codex CLI, Python, an API key or another application.

## Run and sign in

On native Windows, open PowerShell in this repository and run:

```powershell
cargo run --release -p deckcraft -- --sample
```

Open **ChatGPT** in the title bar, click **Continue with ChatGPT**, and complete authorization in
your browser. Return to DeckCraft; it verifies the sign-in, stores the credentials privately and
loads the models available to your account. The browser must run on the same computer because
sign-in returns to a temporary `127.0.0.1` callback. Cancel and retry if the browser flow times out.

Login persists across restarts. **Continue with ChatGPT** reuses a valid saved login, refreshing
it when needed. Account/workspace eligibility, model access and usage limits still apply.

## What the assistant can do

The assistant can discover and execute every registered DeckCraft engine command, plus supported
programmatic view and slide-show controls. This includes creating, rearranging and deleting slides;
adding, editing, moving, resizing, rotating, grouping, aligning and ordering objects; text and font
formatting; shapes, tables, charts, images and media; notes, themes, masters and layouts; and
animations and transitions. Edits go through the normal engine validation and Undo history.

It can inspect structured presentation data and receive rendered slide images to check object
placement, overlap, contrast and text fit. Batches make larger tasks efficient. Example requests:

- “Create a three-stage process diagram, align its cards and make their spacing equal.”
- “Move the selected objects to the right, resize the chart and keep the title readable.”
- “Apply a consistent theme and title style to every slide, then check the rendered layouts.”
- “Use `C:\Reports\brief.docx` as source material for five slides with speaker notes.”

Tools cover features DeckCraft has implemented. They do not implement missing PowerPoint features.
The assistant should report unsupported commands or model capabilities rather than claim success.

| Tool | Purpose |
|---|---|
| `deckcraft_commands` | Discover command IDs, parameters and current enablement; search and paginate |
| `deckcraft_inspect` | Inspect the presentation, slides, shapes, masters and layouts; narrow with JSON pointers |
| `deckcraft_execute` | Run a discovered engine or supported programmatic UI command |
| `deckcraft_batch` | Run up to 32 commands in order; stop on the first error and report completed actions |
| `deckcraft_render` | Inspect an actual rendered slide image without changing the deck |
| `deckcraft_interact` | Editor pointer gestures, keys, text and tool selection in slide coordinates |
| `deckcraft_read_file` | Read user-referenced local files on the background worker without opening them over the deck |

Coordinates and dimensions are **slide points**. Object and slide IDs identify content independently
of the mouse position. Parameter dialogs and file pickers are excluded from assistant calls: the
assistant must provide the documented parameters. Shell commands and operating-system desktop
control are not exposed. Save/export/open/close actions should follow your request; closing unsaved
work is refused unless an explicit `force:true` is supplied for a user request to discard it.

## Web search

Web search is **enabled by default**. Open **ChatGPT → ⋯ → ChatGPT settings** and clear
**Enable web search** to disable it. The setting persists across restarts and applies to the next
message, including within an existing conversation. Enabling search makes the tool available;
it does not force a search for every document edit. Search annotations become clickable source
links in the formatted answer. Availability depends on the signed-in account and selected model.
If the provider refuses the tool, choose an eligible model or disable search; DeckCraft does not
silently claim a successful search.

## Referenced files

Provide a file's exact path in your own chat message, preferably inside backticks or quotation marks.
The file-reading tool checks that the path was mentioned in your chat; a path found only inside a
document, file or web result does not grant access. Paths may point anywhere your operating-system
account can read; there is no restriction to the presentation's folder. Existing automation-root
restrictions, if configured, still apply. Relative paths resolve against the saved presentation's
folder, or the app's working directory when the presentation has no path.

Reading runs on the background worker and leaves the active presentation intact. Auto mode extracts
text from native/PPTX decks and Word/ODT packages, reads UTF-8/UTF-16 text, and returns paged base64
bytes for other binary formats. Binary bytes are **not** universal text extraction: PDFs, legacy
binary Office files and arbitrary media are not promised semantic extraction. The model must not
claim it understood a binary file merely because bytes were returned. It can insert supported
images/media through the normal presentation commands and inspect their rendered appearance.

Reads are paginated (up to 32 KiB per chunk); the assistant follows `nextOffset` while `hasMore` is
true. Text extraction limits are 32 MB per input package, 8 MB per Word/ODT XML part and 2 MB of
extracted text. Larger files remain readable through paged raw-byte mode. Only regular files are
read: devices, pipes and directories are refused. Clearing the conversation, changing presentations
or signing out cancels its worker; a cancelled read cannot continue dispatching document actions.

## Add selection to chat

Select text or objects, then click **Add to Chat** on the floating selection toolbar or right-click
and choose **Add to Chat**. With no objects selected, the command attaches the current slide.
The composer shows a compact reference link, not the full captured text or object data.

Click a reference to go to its source slide/master/layout and select the referenced objects or text.
Hover it to reveal **×**, or focus the remove control with the keyboard, to remove it before sending.
Sent messages retain clickable references. References use stable slide/master/layout and shape IDs,
so moving a slide or object does not redirect a link to the object now occupying its old position.
A removed object produces an error; changed text falls back to its containing object when its
original range cannot be restored. Hidden objects are not silently made visible by following a link.

Attach up to eight references with at most 32,000 bytes of context in total. Draft attachments are
preserved through browser sign-in. References and chat history are in memory only, are excluded
from saved presentations/UI preferences, and reset when the active presentation changes or the app
restarts. Collapsing the sidebar keeps them.

## Conversation controls and formatted replies

**⋯ → Clear conversation** clears messages and attachments, cancels pending actions and retains
login. **Sign out** also removes the saved runtime credentials and attempts remote session revocation.
Completed edits remain in the presentation; use normal Undo to reverse them. An HTTPS operation
already in progress may finish in the background, but a cancelled worker cannot apply later edits.

Assistant replies render Markdown headings, emphasis, lists, quotes, tables, links and code blocks
with copy controls. User messages remain literal text. Provider citation markers are converted into
clickable web links. Model-provided images are not fetched automatically from URLs or local paths.
This is a native Markdown interface, not an embedded ChatGPT website.

## Optional authentication JSON helper

Normal browser sign-in handles JSON creation/import automatically. For an advanced manual export,
run the DeckCraft-owned Rust helper from this repository:

```powershell
cargo run -p deckcraft-ui-egui --bin deckcraft-chatgpt-login
```

Complete authorization and wait for **Sign-in verified**. The helper prints the protected export
location without printing its contents. On Windows, open the default export with:

```powershell
notepad "$env:APPDATA\DeckCraft\chatgpt-login\deckcraft-account.json"
```

Open **ChatGPT → ⋯ → ChatGPT settings → Advanced: import authentication JSON**, paste the complete
JSON into **Protected credential file JSON**, and choose **Import and connect**. Close the editor
and clear the clipboard afterwards. Append `-- --no-browser` to print a sign-in URL for manual
opening on the same machine; use `-- --new-account` for a fresh account/workspace registration.
The helper's `-- --help` starts no sign-in or network activity.

## Credential storage and privacy

Profiles are outside the repository:

| Platform | Profile |
|---|---|
| Windows | `%APPDATA%\DeckCraft\` |
| macOS | `~/Library/Application Support/DeckCraft/` |
| Linux/BSD | `$XDG_CONFIG_HOME/deckcraft/` or `~/.config/deckcraft/` |

Runtime credentials are in `chatgpt-direct/account.json`; the optional helper export is separate,
in `chatgpt-login/deckcraft-account.json`. DeckCraft uses its own registration and profile.
Credentials are protected local plaintext, written atomically
with current-user Windows permissions or Unix mode 0600. Keep profile folders and backups private.
Never share or commit credential JSON. Refresh operations are serialized across processes and rotating
refresh tokens are saved atomically. Only one application should refresh a shared credential session.

ChatGPT is optional and online: questions, selected excerpts, requested file contents, necessary
presentation/tool context and rendered previews go to OpenAI. Requests use `store=false` and streaming;
this does not promise zero provider retention. Login grants are verified using OpenAI signing keys,
issuer, audience, expiry, nonce and required scopes. OAuth errors are sanitized; tokens are not logged.
HTTPS uses the operating system's certificate trust store.

## Verification and limitations

Validation on 2026-10-15: all seven `cargo xtask ci` gates passed on native Windows with the
settings below: 1,699 workspace tests passed (including 29 ChatGPT tests), with seven ignored.
The ignored ChatGPT GPU preview also passed separately and its screenshots were inspected.

Offline tests cover the authorization flow and private storage, bounded response parsing, completed-
response gating, web-search preference/payload/citations, real command edits and Undo, batch failures,
selection references, file reads, stale ownership checks and native UI behavior. Headless screenshots
use synthetic slides and openly licensed fonts. Live browser sign-in, account-specific model/search
availability and an actual model editing turn require an interactive account check.

For native Windows validation, select the desktop application's graphics backend/compiler and run
headless graphics tests serially:

```powershell
$env:WGPU_BACKEND = "dx12"
$env:WGPU_DX12_COMPILER = "fxc"
$env:RUST_TEST_THREADS = "1"
cargo xtask ci
```

Parallel headless UI runs on the validation machine faulted in Windows' `d3d10warp.dll`;
serial execution passed the full UI suite. The screenshot fixture uses the embedded Ubuntu font,
synthetic slide content and no account. To regenerate its ignored local previews under `target/`:

```powershell
cargo test -p deckcraft-ui-egui chatgpt::tests::presentation_chat_preview -- --ignored
```

The browser-hosted WASM app displays desktop availability information; native credentials, file reads
and HTTPS transport are not compiled into its runtime.

Official protocol references: [sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in),
[models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference),
[sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions),
[web search and citations](https://developers.openai.com/api/docs/guides/tools-web-search).

## Revision history

| Date | Change |
|---|---|
| 2026-10-15 | Initial DeckCraft ChatGPT sidebar, browser login, full command tools, batches, visual inspection, web search, file reads and selection references |
