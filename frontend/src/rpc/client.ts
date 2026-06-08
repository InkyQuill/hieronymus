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

  constructor(private readonly proc: ChildProcessWithoutNullStreams) {
    const lines = createInterface({input: proc.stdout});
    lines.on('line', (line) => this.receive(line));
    proc.stderr.on('data', (chunk) => {
      process.stderr.write(chunk);
    });
  }

  request(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>> {
    const id = String(this.nextId++);
    const payload = JSON.stringify({id, method, params});
    this.proc.stdin.write(`${payload}\n`);
    return new Promise((resolve, reject) => {
      this.pending.set(id, {resolve, reject});
    });
  }

  close(): void {
    this.proc.stdin.end();
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
}

export function createBridgeClient(command: string): JsonRpcClient {
  const proc = spawn(command, ['tui-bridge'], {
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  return new JsonRpcClient(proc);
}
