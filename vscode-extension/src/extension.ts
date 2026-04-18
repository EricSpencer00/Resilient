// RES-204: VS Code extension entry point. Launches the Resilient
// binary as a Language Server over stdio and wires it up to VS
// Code's LanguageClient.
//
// Activation is deferred until a `.rs` file with languageId
// `resilient` opens (see `package.json` activationEvents). The
// client is torn down in `deactivate`; VS Code calls that on
// editor shutdown and when the user disables the extension.

import { workspace, ExtensionContext, window } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: ExtensionContext): void {
  const config = workspace.getConfiguration("resilient");
  const serverPath = config.get<string>("serverPath", "resilient");
  const serverArgs = config.get<string[]>("serverArgs", ["--lsp"]);

  // Spawn the same binary for both "run" (production) and
  // "debug" (F5 in the extension dev host). The Resilient server
  // has no debug-specific mode, so both point at the same place.
  const serverOptions: ServerOptions = {
    run: {
      command: serverPath,
      args: serverArgs,
      transport: TransportKind.stdio,
    },
    debug: {
      command: serverPath,
      args: serverArgs,
      transport: TransportKind.stdio,
    },
  };

  const clientOptions: LanguageClientOptions = {
    // Activate for any file tagged `resilient` (the language id
    // is registered in package.json → contributes.languages).
    documentSelector: [
      { scheme: "file", language: "resilient" },
    ],
    synchronize: {
      // File watchers for .rs files in the workspace so the
      // server can refresh workspace symbol indexes on save.
      fileEvents: workspace.createFileSystemWatcher("**/*.rs"),
    },
    // RES-204: surface the trace setting so users can opt into
    // verbose LSP traffic via `resilient.trace.server`.
    traceOutputChannel: window.createOutputChannel("Resilient LSP"),
  };

  client = new LanguageClient(
    "resilient",
    "Resilient Language Server",
    serverOptions,
    clientOptions,
  );

  // Start asynchronously — surface any spawn failure as an error
  // toast rather than bubbling up into the activation error path,
  // which VS Code reports without context.
  client.start().catch((err: unknown) => {
    const msg = err instanceof Error ? err.message : String(err);
    window.showErrorMessage(
      `Resilient: failed to start Language Server at ` +
        `\`${serverPath}\`: ${msg}. Set ` +
        "`resilient.serverPath` to point at your build.",
    );
  });

  // Keep a handle so `deactivate` can stop cleanly.
  context.subscriptions.push({
    dispose: () => {
      if (client) {
        void client.stop();
      }
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
