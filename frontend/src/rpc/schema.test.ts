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
        {
          name: provider,
          display_name: displayName,
          requires_api_key: true,
          supports_api_path: true,
        },
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

  it('parses the current Python config bootstrap payload shape', () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        data_root: '/tmp/hieronymus',
        config_root: '/tmp/hieronymus/config',
        settings_path: '/tmp/hieronymus/config/settings.toml',
      },
      provider_choices: [
        {
          display_name: 'OpenAI compatible',
          name: 'openai',
          requires_api_key: true,
          supports_api_path: true,
        },
        {
          display_name: 'Gemini',
          name: 'gemini',
          requires_api_key: true,
          supports_api_path: false,
        },
        {
          display_name: 'Anthropic',
          name: 'anthropic',
          requires_api_key: true,
          supports_api_path: false,
        },
      ],
      selected_provider: 'openai',
      draft: {dreaming: {active_provider: 'openai'}, providers: {}},
      form_values: {provider: {}, dreaming: {}},
      validation: {ok: true, errors: []},
      check_result: {},
      suggestions: {},
      detail: '',
    });

    expect(payload.suggestions).toEqual({});
    expect(payload.detail).toBe('');
    expect(payload.provider_choices[0].requires_api_key).toBe(true);
  });

  it('accepts an empty config detail payload', () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        settings_path: '/tmp/hieronymus/config/settings.toml',
      },
      provider_choices: [
        {
          display_name: 'OpenAI compatible',
          name: 'openai',
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: 'openai',
      draft: {dreaming: {active_provider: 'openai'}, providers: {}},
      form_values: {provider: {}, dreaming: {}},
      validation: {ok: true, errors: []},
      suggestions: {},
      detail: {},
    });

    expect(payload.detail).toEqual({});
  });

  it('rejects config provider choices outside supported families', () => {
    expect(() =>
      ConfigBootstrapSchema.parse({
        config_paths: {},
        provider_choices: [
          {
            name: 'deterministic',
            display_name: 'Deterministic',
            requires_api_key: false,
            supports_api_path: false,
          },
        ],
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
