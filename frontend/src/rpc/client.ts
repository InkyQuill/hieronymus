import {spawn, type ChildProcessWithoutNullStreams} from 'node:child_process';
import {createInterface} from 'node:readline';
import {RpcResponseSchema} from './schema.js';

type Pending = {
  resolve: (value: Record<string, unknown>) => void;
  reject: (error: Error) => void;
};

export class JsonRpcClient {
  private nextId = 1;
  private readonly pending = new Map<string, Pending>();
  private closedError: Error | undefined;

  constructor(private readonly proc: ChildProcessWithoutNullStreams) {
    const lines = createInterface({input: proc.stdout});
    lines.on('line', (line) => this.receiveLine(line));
    proc.stderr.on('data', (chunk) => {
      process.stderr.write(chunk);
    });
    proc.stdin.on('error', (error) => {
      this.rejectAll(new Error(`bridge stdin error: ${error.message}`));
    });
    proc.on('error', (error) => {
      this.closeWithError(new Error(`bridge process error: ${error.message}`));
    });
    proc.on('exit', (code, signal) => {
      this.closeWithError(processClosedError('exited', code, signal));
    });
    proc.on('close', (code, signal) => {
      this.closeWithError(processClosedError('closed', code, signal));
    });
  }

  request(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>> {
    if (this.closedError) {
      return Promise.reject(this.closedError);
    }
    const id = String(this.nextId++);
    const payload = JSON.stringify({id, method, params});
    return new Promise((resolve, reject) => {
      this.pending.set(id, {resolve, reject});
      const rejectWrite = (error: unknown) => {
        if (this.pending.delete(id)) {
          reject(asError('bridge stdin write failed', error));
        }
      };
      try {
        this.proc.stdin.write(`${payload}\n`, (error?: Error | null) => {
          if (error) {
            rejectWrite(error);
          }
        });
      } catch (error) {
        rejectWrite(error);
      }
    });
  }

  close(): void {
    this.proc.stdin.end();
  }

  private receiveLine(line: string): void {
    try {
      this.receive(line);
    } catch (error) {
      this.rejectAll(asError('invalid bridge response', error));
    }
  }

  private receive(line: string): void {
    const response = RpcResponseSchema.parse(JSON.parse(line));
    const {id} = response;
    if (id === null) {
      if (!response.ok) {
        throw new Error(response.error.message);
      }
      return;
    }
    const pending = this.pending.get(id);
    if (!pending) {
      return;
    }
    this.pending.delete(id);
    if (response.ok) {
      pending.resolve(response.result);
    } else {
      pending.reject(new Error(response.error.message));
    }
  }

  private closeWithError(error: Error): void {
    this.closedError ??= error;
    this.rejectAll(error);
  }

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) {
      pending.reject(error);
    }
    this.pending.clear();
  }
}

export function createBridgeClient(command: string): JsonRpcClient {
  const proc = spawn(command, ['tui-bridge'], {
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  return new JsonRpcClient(proc);
}

function asError(prefix: string, error: unknown): Error {
  if (error instanceof Error) {
    return new Error(`${prefix}: ${error.message}`);
  }
  return new Error(`${prefix}: ${String(error)}`);
}

function processClosedError(
  event: 'closed' | 'exited',
  code: number | null,
  signal: NodeJS.Signals | null,
): Error {
  const detail = signal ? `signal ${signal}` : `code ${code ?? 'unknown'}`;
  return new Error(`bridge process ${event} before response (${detail})`);
}
