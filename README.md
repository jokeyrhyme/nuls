# nuls

Language Server Protocol implementation for nushell

## status

- the official [nushell](http://www.nushell.sh/) project
  (from version [0.79](https://www.nushell.sh/blog/2023-04-25-nushell_0_79.html), onwards)
  is where the language-specific smarts are implemented,
  e.g. `nu --ide-hover`

- the official [extension for Visual Studio Code](https://github.com/nushell/vscode-nushell-lang)
  is an IDE-specific wrapper around `nu --ide-hover`, etc

- similarly, `nuls` (this project) is a wrapper around the `nu --ide-hover`, etc,
  but implements the [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)

## project scope

- `nuls` aims to have all the same LSP-powered features as the Visual Studio Code extension,
  but also working in any other IDE/editor that can connect to a language server,
  e.g. [`helix`](https://helix-editor.com/), [`lapce`](https://lapce.dev/), [`neovim`](https://neovim.io/), [`zed`](https://zed.dev/), etc

- for now, please keep feature requests and bug reports focused on this goal

- functionality that is not supported by upstream `nu --ide-...` is out-of-scope

- functionality in `vscode-nushell-lang` that goes beyond LSP is out-of-scope

## roadmap

(in no particular order, and open to suggestions)

### parity with [extension for Visual Studio Code](https://github.com/nushell/vscode-nushell-lang)

- [x] [textDocument/hover](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover) -> `nu --ide-hover`
- [x] [textDocument/completion](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_completion) -> `nu --ide-complete`
- [x] [textDocument/definition](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_definition) -> `nu --ide-goto-def`
- [x] [textDocument/didChange](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_didChange),
      [textDocument/didClose](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_didClose),
      and [textDocument/didOpen](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_didOpen)
- [x] [textDocument/publishDiagnostics](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_publishDiagnostics) -> `nu --ide-check`
- [x] [workspace/configuration](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#workspace_configuration)
- [ ] [textDocument/inlayHint](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_inlayHint) -> `nu --ide-check`
- [ ] [workspace/didChangeConfiguration](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#workspace_didChangeConfiguration)
- [ ] raise a PR for `vscode-nushell-lang` to replace its wrapper/glue code with `nuls`

### stretch goals

- [ ] [textDocument/diagnostic](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_pullDiagnostics) -> `nu --ide-check`
- [ ] [textDocument/formatting](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_formatting) -> [`nufmt`](https://github.com/nushell/nufmt)
- [ ] [window/workDoneProgress/create](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#workspace_didChangeConfiguration) and [window/workDoneProgress/cancel](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#window_workDoneProgress_cancel)

## getting started

- it's still an open question as to how this project will be distributed and in what form

- we'd like to tackle those specific in close collaboration with the nushell maintainers ([#3](https://github.com/jokeyrhyme/nuls/issues/3)),
  perhaps once we're a little closer to integation with the Visual Studio Code extension ([#9](https://github.com/jokeyrhyme/nuls/issues/9))

### installation

1. you'll need a [stable Rust toolchain](https://www.rust-lang.org/)

2. `cargo install --git https://github.com/jokeyrhyme/nuls.git --locked`

### `helix` (23.05)

- (optional) follow https://github.com/nushell/tree-sitter-nu/blob/main/installation/helix.md for the treesitter grammar

- add the following to your languages.toml:

  ```toml
  [[language]]
  name = "nu"
  auto-format = false
  comment-token = "#"
  file-types = [ "nu" ]
  language-server = { command = "path/to/nuls" }
  roots = []
  scope = "source.nu"
  shebangs = ["nu"]
  ```

### `helix` with [multiple language servers per language](https://github.com/helix-editor/helix/pull/2507)

recent-enough commits of `helix` now include the nushell grammar and language definition out-of-the-box,
so all we need to do here tell it to use `nuls`

- add the following to your languages.toml:

  ```toml
  [language-server.nuls]
  command = "nuls" # or "some/path/to/nuls"

  [[language]]
  name = "nu"
  language-servers = [ "nuls" ]
  ```

## see also

- http://www.nushell.sh/
- https://github.com/nushell/vscode-nushell-lang
- https://github.com/nushell/vscode-nushell-lang/issues/117
- https://github.com/nushell/tree-sitter-nu
- https://github.com/tree-sitter/tree-sitter
- https://microsoft.github.io/language-server-protocol/
