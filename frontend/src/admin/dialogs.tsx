export type DialogKind =
  | "add"
  | "edit"
  | "filter"
  | "delete"
  | "merge"
  | "split"
  | "supersede"
  | "none";

export type DialogState = {
  kind: DialogKind;
  error: string;
};

export const closedDialog: DialogState = { kind: "none", error: "" };
