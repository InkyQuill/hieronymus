import {EventEmitter} from 'node:events';
import {Readable, Writable} from 'node:stream';
import {describe, expect, it} from 'vitest';
import {JsonRpcClient} from './client.js';

class FakeProcess extends EventEmitter {
  stdin: Writable;
  stdout: Readable;
  stderr: Readable;
  writes: string[] = [];

  constructor() {
    super();
    this.stdin = new Writable({
      write: (chunk, _encoding, callback) => {
        this.writes.push(String(chunk));
        callback();
      },
    });
    this.stdout = new Readable({read() {}});
    this.stderr = new Readable({read() {}});
  }
}

describe('JsonRpcClient', () => {
  it('writes request envelopes and resolves matching response', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('config.bootstrap', {});
    expect(proc.writes[0]).toContain('"method":"config.bootstrap"');
    proc.stdout.push('{"id":"1","ok":true,"result":{"ready":true}}\n');

    await expect(pending).resolves.toEqual({ready: true});
  });

  it('rejects backend error envelopes', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('admin.edit_crystal', {});
    proc.stdout.push(
      '{"id":"1","ok":false,"error":{"code":"validation_error","message":"text must not be empty"}}\n',
    );

    await expect(pending).rejects.toThrow('text must not be empty');
  });

  it('rejects pending requests when stdout contains malformed JSON', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('config.bootstrap', {});
    proc.stdout.push('not json\n');

    await expect(pending).rejects.toThrow('invalid bridge response');
  });

  it('rejects pending requests when the child process emits an error', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('config.bootstrap', {});
    proc.emit('error', new Error('spawn ENOENT'));

    await expect(pending).rejects.toThrow('bridge process error: spawn ENOENT');
  });

  it('rejects pending requests when the child process closes before response', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('config.bootstrap', {});
    proc.emit('close', 1, null);

    await expect(pending).rejects.toThrow('bridge process closed before response');
  });
});
