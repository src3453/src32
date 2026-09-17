import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';
import * as path from 'path';
let client: LanguageClient | undefined;
export function activate(context: vscode.ExtensionContext) {
  const python = vscode.workspace.getConfiguration('sol').get<string>('languageServer.pythonPath') || 'python3';
  const server = path.join(context.extensionPath, 'server', 'sol_language_server.py');
  const options: ServerOptions = { command: python, args: [server], options: { env: process.env } };
  const clientOptions: LanguageClientOptions = { documentSelector: [{ scheme: 'file', language: 'sol' }] };
  client = new LanguageClient('solLanguageServer', 'sol Language Server', options, clientOptions);
  client.start().catch(err => vscode.window.showErrorMessage(`sol language server failed to start: ${err}`));
}
export function deactivate() { return client?.stop(); }
