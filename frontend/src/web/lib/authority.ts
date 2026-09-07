export type ClaimTarget = { source: "short_term" | "crystal" | "facet" | "rag_chunk"; id: number };
export type Rule = { id: number; revision: number; canonical: string };
export type SourceChoice = { id: number; series_id: number; selected_text: string; context?: string; source_identity: string; start: number; chapter: string | null; rules: Rule[] };
export type CorrectionOptions = { series: { id: number; title: string }[]; sources: SourceChoice[] };
export type Applicability = Record<string, unknown>;
export type Claim = { claim_id: number; revision: number; text: string; applicability: Applicability };
export type Selection = { series_id: number; expected_revision: number; source_language: string; target_language: string; claims: Claim[]; source: null | { reference: Record<string, unknown>; selected_text: string; binding: { applicability: Applicability } }; rule: Rule | null };
export class AuthorityError extends Error {
  constructor(public status: number, public detail: unknown) {
    super(typeof detail === "object" && detail !== null && "RevisionConflict" in detail
      ? "This selection changed. Refresh the selection and review your correction."
      : typeof detail === "string" ? detail : "The correction could not be applied.");
  }
}
async function post<T>(route: string, body: unknown): Promise<T> {
  const response = await fetch(`/api/authority/${route}`, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const result = await response.json();
  if (!response.ok) throw new AuthorityError(response.status, result.error);
  return result as T;
}
export const correctionOptions = (target?: ClaimTarget) => post<CorrectionOptions>("options", target ? { target } : {});
export const correctionSelection = (body: { series_id: number; target?: ClaimTarget; source_evidence_id?: number; rule_id?: number }) => post<Selection>("selection", body);
export type CorrectionResponse = { Applied?: { receipt: { decision_id: string } }; Replayed?: { receipt: { decision_id: string } }; Tentative?: { reasons: string[] }; status?: string; reasons?: string[]; detail?: string };
export const submitCorrection = (body: Record<string, unknown>) => post<CorrectionResponse>("correct", body);
