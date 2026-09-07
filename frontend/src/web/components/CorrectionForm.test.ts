import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import CorrectionForm from "./CorrectionForm.svelte";
import { correctionOptions, correctionSelection, submitCorrection, AuthorityError } from "../lib/authority";
vi.mock("../lib/authority", async (original) => ({ ...(await original<object>()), correctionOptions: vi.fn(), correctionSelection: vi.fn(), submitCorrection: vi.fn() }));
const app = { series_id: 1, chapter_key: "1" };
const selection = { series_id: 1, expected_revision: 7, source_language: "en", target_language: "ru", claims: [{ claim_id: 1, revision: 2, text: "First claim", applicability: app }, { claim_id: 2, revision: 3, text: "Second claim", applicability: app }], source: null, rule: null };
beforeEach(() => {vi.mocked(correctionOptions).mockResolvedValue({ series: [{id:1,title:"Book"}],sources:[] });vi.mocked(correctionSelection).mockResolvedValue(selection);vi.mocked(submitCorrection).mockResolvedValue({Applied:{receipt:{decision_id:"receipt"}}});});
test("multiple claims require a deliberate choice and preserve displayed revisions",async()=>{
 const user=userEvent.setup();render(CorrectionForm,{props:{target:{source:"short_term",id:4},onclose:vi.fn()}});
 await screen.findByText("First claim");expect((screen.getByRole("button",{name:"Apply correction"}) as HTMLButtonElement).disabled).toBe(true);
 await user.click(screen.getByLabelText(/Second claim/));await user.click(screen.getByRole("button",{name:"Apply correction"}));
 await screen.findByText("Correction applied. This claim is now marked incorrect.");
 expect(submitCorrection).toHaveBeenCalledWith(expect.objectContaining({expected_revision:7,selected_claims:[{id:2,revision:3}],structured:{kind:"invalidate"},applicability:app}));
});
test("stale selection remains visible and is never silently refreshed",async()=>{
 vi.mocked(submitCorrection).mockRejectedValue(new AuthorityError(409,{RevisionConflict:{current_revision:8}}));
 const user=userEvent.setup();render(CorrectionForm,{props:{target:{source:"short_term",id:4},onclose:vi.fn()}});
 await screen.findByText("First claim");await user.click(screen.getByLabelText(/First claim/));await user.click(screen.getByRole("button",{name:"Apply correction"}));
 await screen.findByText(/This selection changed/);expect(correctionSelection).toHaveBeenCalledTimes(1);
});
test("tentative outcome is visible without success claim",async()=>{
 vi.mocked(submitCorrection).mockResolvedValue({status:"tentative",reasons:["UnknownOrder"]});
 const user=userEvent.setup();render(CorrectionForm,{props:{target:{source:"short_term",id:4},onclose:vi.fn()}});await screen.findByText("First claim");await user.click(screen.getByLabelText(/First claim/));await user.click(screen.getByRole("button",{name:"Apply correction"}));
 await waitFor(()=>expect(screen.getByRole("status").textContent).toContain("Not applied"));
});

test("explicit retry recovers failed option loading",async()=>{
 vi.mocked(correctionOptions).mockRejectedValueOnce(new TypeError("Failed to fetch"));
 const user=userEvent.setup();render(CorrectionForm,{props:{target:{source:"short_term",id:4},onclose:vi.fn()}});
 await screen.findByText("Failed to fetch");await user.click(screen.getByRole("button",{name:"Refresh selection"}));
 await screen.findByText("First claim");expect(correctionOptions).toHaveBeenCalledTimes(2);
});

test("rendering requires actual occurrence and rule then displays current replacement",async()=>{
 const reference={id:55,kind:"source_passage",content_hash:"unchanged",span_start:10,span_end:14};
 vi.mocked(correctionOptions).mockResolvedValue({series:[{id:1,title:"Book"}],sources:[{id:55,series_id:1,selected_text:"Alex",context:"Alex walks.",source_identity:"source.txt",start:10,chapter:"1",rules:[{id:9,revision:4,canonical:"A"}]}]});
 vi.mocked(correctionSelection).mockImplementation(async input=>({...selection,claims:[],source:{reference,selected_text:"Alex",binding:{applicability:app}},rule:input.rule_id?{id:9,revision:4,canonical:"A"}:null}));
 const user=userEvent.setup();render(CorrectionForm,{props:{onclose:vi.fn()}});
 await screen.findByRole("option",{name:/Alex walks/});await user.selectOptions(screen.getByLabelText("Source occurrence"),"55");
 await screen.findByText(/Selected source/);await user.type(screen.getByLabelText("Correct rendering"),"B");
 expect((screen.getByRole("button",{name:"Apply correction"}) as HTMLButtonElement).disabled).toBe(true);
 await user.selectOptions(screen.getByLabelText("Current rendering to replace"),"9");
 await waitFor(()=>expect((screen.getByRole("button",{name:"Apply correction"}) as HTMLButtonElement).disabled).toBe(false));await user.click(screen.getByRole("button",{name:"Apply correction"}));
 await screen.findByText("Correction applied. Current rendering: B");expect(screen.getByLabelText("Previous rendering (frozen selection)")).toBeTruthy();expect(submitCorrection).toHaveBeenCalledWith(expect.objectContaining({expected_revision:7,selected_sources:[reference],selected_rule:{id:9,revision:4},structured:{kind:"rendering",canonical:"B"}}));
});
test("network retry retains the exact correction request",async()=>{
 vi.mocked(submitCorrection).mockRejectedValueOnce(new TypeError("Failed to fetch"));const user=userEvent.setup();render(CorrectionForm,{props:{target:{source:"short_term",id:4},onclose:vi.fn()}});await screen.findByText("First claim");await user.click(screen.getByLabelText(/First claim/));await user.click(screen.getByRole("button",{name:"Apply correction"}));await screen.findByText("Failed to fetch");await user.click(screen.getByRole("button",{name:"Apply correction"}));await screen.findByText("Correction applied. This claim is now marked incorrect.");expect(vi.mocked(submitCorrection).mock.calls[0][0]).toEqual(vi.mocked(submitCorrection).mock.calls[1][0]);
});
