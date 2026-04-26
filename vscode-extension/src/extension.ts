import { workspace, ExtensionContext, window, commands } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: ExtensionContext): void {
  const config = workspace.getConfiguration("resilient");
  const serverPath = config.get<string>("serverPath", "rz");
  const serverArgs = config.get<string[]>("serverArgs", ["--lsp"]);

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
    documentSelector: [{ scheme: "file", language: "resilient" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.rz"),
    },
    traceOutputChannel: window.createOutputChannel("Resilient LSP"),
  };

  client = new LanguageClient(
    "resilient",
    "Resilient Language Server",
    serverOptions,
    clientOptions,
  );

  client.start().catch((err: unknown) => {
    const msg = err instanceof Error ? err.message : String(err);
    window.showErrorMessage(
      `Resilient: failed to start Language Server at ` +
        `\`${serverPath}\`: ${msg}. Set ` +
        "`resilient.serverPath` to point at your build.",
    );
  });

  const runFile = commands.registerCommand("resilient.runFile", () => {
    const editor = window.activeTextEditor;
    if (!editor) {
      window.showErrorMessage("Resilient: no active editor.");
      return;
    }
    const filePath = editor.document.uri.fsPath;
    const bin = workspace
      .getConfiguration("resilient")
      .get<string>("serverPath", "rz");

    let terminal = window.terminals.find((t) => t.name === "Resilient");
    if (!terminal || terminal.exitStatus !== undefined) {
      terminal = window.createTerminal("Resilient");
    }
    terminal.show(true);
    terminal.sendText(`"${bin}" "${filePath}"`);
  });

  context.subscriptions.push(runFile, {
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
