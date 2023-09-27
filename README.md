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
  but working in any IDE/editor that can connect to a language server,
  e.g. [`helix`](https://helix-editor.com/), [`neovim`](https://neovim.io/), etc

- for now, please keep feature requests and bug reports focused on this goal

- functionality that is not supported by upstream `nu --ide-...` is out-of-scope

- functionality in `vscode-nushell-lang` that goes beyond LSP is out-of-scope

## roadmap

(in no particular order, and open to suggestions)

- [x] [textDocument/hover](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover) -> `nu --ide-hover`
- [x] [textDocument/completion](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_completion) -> `nu --ide-complete`
- [x] [textDocument/definition](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_definition) -> `nu --ide-goto-def`
      (navigates to file containing the definition)
- [ ] [textDocument/definition](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_definition) -> `nu --ide-goto-def`
      (navigates to precise row and column for the definition)
- [ ] [textDocument/inlayHint](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_inlayHint) -> `nu --ide-check`
- [ ] [textDocument/publishDiagnostics](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_publishDiagnostics) -> `nu --ide-check`
- [ ] [textDocument/didOpen](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_synchronization), etc
- [ ] [textDocument/formatting](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_formatting) -> [`nufmt`](https://github.com/nushell/nufmt)
- [ ] raise a PR for `vscode-nushell-lang` to replace its wrapper/glue code with `nuls`

## getting started

### `helix` (23.05)

- (optional) follow https://github.com/nushell/tree-sitter-nu/blob/main/installation/helix.md for the treesitter grammar

- add the following to your languages.toml:

  ```
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

## see also

- http://www.nushell.sh/
- https://github.com/nushell/vscode-nushell-lang
- https://github.com/nushell/vscode-nushell-lang/issues/117
- https://github.com/nushell/tree-sitter-nu
- https://microsoft.github.io/language-server-protocol/
