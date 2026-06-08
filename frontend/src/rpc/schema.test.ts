import {describe, expect, it} from 'vitest';
import {AdminSnapshotSchema, ConfigBootstrapSchema, RpcResponseSchema} from './schema.js';

describe('runtime schemas', () => {
  it.each([
    ['openai', 'OpenAI compatible'],
    ['gemini', 'Gemini'],
    ['anthropic', 'Anthropic'],
  ] as const)('parses config bootstrap payload for %s provider selector', (provider, displayName) => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        config_root: '/tmp/h',
        settings_path: '/tmp/h/settings.toml',
        database_path: '/tmp/h/hieronymus.sqlite3',
      },
      provider_choices: [
        {name: provider, display_name: displayName, supports_api_path: true},
      ],
      selected_provider: provider,
      draft: {dreaming: {active_provider: provider}, providers: {}},
      form_values: {provider: {model: 'gpt-4.1-mini'}, dreaming: {}},
      validation: {ok: true, errors: []},
      suggestions: {provider, models: ['gpt-4.1-mini'], source: 'defaults', error: ''},
      detail: {title: `${provider} dreaming provider`, fields: [], errors: []},
    });

    expect(payload.selected_provider).toBe(provider);
  });

  it('rejects config provider choices outside supported families', () => {
    expect(() =>
      ConfigBootstrapSchema.parse({
        config_paths: {},
        provider_choices: [{name: 'deterministic', display_name: 'Deterministic', supports_api_path: false}],
        selected_provider: 'deterministic',
        draft: {dreaming: {}, providers: {}},
        form_values: {provider: {}, dreaming: {}},
        validation: {ok: true, errors: []},
        suggestions: {provider: 'deterministic', models: [], source: 'defaults', error: ''},
        detail: {title: '', fields: [], errors: []},
      }),
    ).toThrow();
  });

  it('parses admin snapshots', () => {
    const snapshot = AdminSnapshotSchema.parse({
      view: 'Crystals',
      rows: [],
      selected: null,
      detail: {title: 'Empty', subtitle: '', body: '', fields: []},
      filters: [],
    });

    expect(snapshot.view).toBe('Crystals');
  });

  it('parses success and error envelopes', () => {
    expect(RpcResponseSchema.parse({id: '1', ok: true, result: {ready: true}}).ok).toBe(true);
    expect(
      RpcResponseSchema.parse({
        id: '1',
        ok: false,
        error: {code: 'validation_error', message: 'text must not be empty'},
      }).ok,
    ).toBe(false);
  });
});
