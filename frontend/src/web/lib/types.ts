export type ProviderProfile = {
  id: string;
  name: string;
  type: "openai" | "google" | "anthropic" | "ollama";
  url: string;
  key_configured: boolean;
  model: string;
  timeout_seconds: number;
};

export type ProviderDraft = {
  id: string;
  name: string;
  type: ProviderProfile["type"];
  url: string;
  key: string;
  timeout_seconds: string;
};
