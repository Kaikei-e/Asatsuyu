import * as path from "path";
import {
  commands,
  env,
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

  void client.start();

  // Walkthrough helper: open the starter buffer as an untitled Asatsuyu file.
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

  // Walkthrough helper: open the online syntax guide from the extension.
  const openSyntaxGuide = commands.registerCommand(
    "asatsuyu.openSyntaxGuide",
    async () => {
      await env.openExternal(
        Uri.parse(
          "https://github.com/kaikei/asatsuyu/blob/main/docs/grammar.md"
        )
      );
    }
  );

  context.subscriptions.push(openStarter, openSyntaxGuide);
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
