import * as path from "path";
import {
  commands,
  ExtensionContext,
  window,
  workspace,
  Uri,
} from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: ExtensionContext) {
  // Find the asatsuyu binary. Check settings first, then PATH.
  const config = workspace.getConfiguration("asatsuyu");
  const serverPath: string = config.get("serverPath") || "asatsuyu";

  const serverOptions: ServerOptions = {
    command: serverPath,
    args: ["lsp"],
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "asatsuyu" },
      { scheme: "untitled", language: "asatsuyu" },
    ],
  };

  client = new LanguageClient(
    "asatsuyu",
    "Asatsuyu Language Server",
    serverOptions,
    clientOptions
  );

  client.start();

  // Register the "Open Starter File" command for the walkthrough.
  const openStarter = commands.registerCommand(
    "asatsuyu.openStarter",
    async () => {
      const starterPath = path.join(
        context.extensionPath,
        "media",
        "starter.asty"
      );
      const content = await workspace.fs.readFile(Uri.file(starterPath));
      const doc = await workspace.openTextDocument({
        language: "asatsuyu",
        content: new TextDecoder().decode(content),
      });
      await window.showTextDocument(doc, { preview: false });
    }
  );
  context.subscriptions.push(openStarter);
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
