import React, { useState } from "react";
import { useKeyboard } from "@opentui/react";
import { TextAreaInput, TextInput } from "../ui/TextInput.js";

export type DialogKind =
  | "add"
  | "edit"
  | "rename"
  | "merge"
  | "split"
  | "delete"
  | "none";

export type DialogState = {
  kind: DialogKind;
  error: string;
  entityId?: string | number;
  entityType?: "crystal" | "concept" | "memory";
  initialTitle?: string;
  initialText?: string;
};

export const closedDialog: DialogState = { kind: "none", error: "" };

type DialogProps = {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
};

export function DialogOverlay({ state, onClose, onSubmit }: DialogProps) {
  if (state.kind === "none") {
    return null;
  }

  // Common styles
  const overlayStyle: any = {
    position: "absolute",
    top: 0,
    left: 0,
    width: "100%",
    height: "100%",
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "#000000",
  };

  const modalStyle: any = {
    borderStyle: "double",
    borderColor: "cyan",
    flexDirection: "column",
    padding: 1,
    minWidth: 60,
    backgroundColor: "#141414",
  };

  if (state.kind === "delete") {
    return (
      <DeleteDialog
        state={state}
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
      />
    );
  }

  if (state.kind === "add") {
    return (
      <AddDialog
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
      />
    );
  }

  if (state.kind === "edit") {
    return (
      <EditDialog
        state={state}
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
      />
    );
  }

  if (state.kind === "rename") {
    return (
      <RenameDialog
        state={state}
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
      />
    );
  }

  if (state.kind === "merge") {
    return (
      <MergeDialog
        state={state}
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
      />
    );
  }

  if (state.kind === "split") {
    return (
      <SplitDialog
        state={state}
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
      />
    );
  }

  return null;
}

// 1. Delete Dialog
function DeleteDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
}) {
  useKeyboard((key) => {
    if (key.name === "escape" || key.name === "n" || key.name === "N") {
      onClose();
    } else if (key.name === "enter" || key.name === "y" || key.name === "Y") {
      if (state.entityType === "concept") {
        onSubmit({ concept_id: state.entityId, confirmed: true });
      } else if (state.entityType === "memory") {
        onSubmit({ memory_id: state.entityId, confirmed: true });
      } else {
        onSubmit({ id: state.entityId, confirmed: true });
      }
    }
  });

  return (
    <box {...overlayStyle}>
      <box {...modalStyle} borderColor="red">
        <text fg="red">Confirm Deletion</text>
        <box marginTop={1}>
          <text>
            Are you sure you want to delete this {state.entityType || "item"}?
          </text>
        </box>
        <box marginTop={1} flexDirection="row">
          <text fg="gray">ID: </text>
          <text>{state.entityId}</text>
        </box>
        {state.error ? (
          <box marginTop={1}>
            <text fg="red">{state.error}</text>
          </box>
        ) : null}
        <box marginTop={1} justifyContent="space-between">
          <text fg="gray">[Y] Yes, Delete</text>
          <text fg="gray">[Esc/N] Cancel</text>
        </box>
      </box>
    </box>
  );
}

// 2. Add Dialog
function AddDialog({
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
}: {
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
}) {
  const [type, setType] = useState<"crystal" | "lesson" | "rule" | string>(
    "crystal",
  );
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [tags, setTags] = useState("");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0 = type, 1 = title, 2 = text, 3 = tags

  const handleSubmit = () => {
    onSubmit({
      series_slug: "only-sense-online",
      source_language: "ja",
      target_language: "ru",
      crystal_type: type,
      title,
      text,
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
    });
  };

  useKeyboard((key) => {
    if (key.name === "escape") {
      onClose();
      return;
    }
    if (key.name === "enter") {
      handleSubmit();
      return;
    }
    if (key.name === "up") {
      setFocusedIndex((prev) => Math.max(0, prev - 1));
    } else if (key.name === "down") {
      setFocusedIndex((prev) => Math.min(3, prev + 1));
    } else if (focusedIndex === 0) {
      if (
        key.name === "left" ||
        key.name === "right" ||
        key.name === "space" ||
        key.name === " "
      ) {
        setType((prev) => {
          if (prev === "crystal") return "lesson";
          if (prev === "lesson") return "rule";
          return "crystal";
        });
      }
    }
  });

  return (
    <box {...overlayStyle}>
      <box {...modalStyle}>
        <text fg="cyan">Add New Crystal / Lesson / Rule</text>
        <box marginTop={1} flexDirection="column">
          <box flexDirection="row">
            <text fg={focusedIndex === 0 ? "cyan" : "gray"}>Type: </text>
            <select
              width={18}
              height={3}
              focused={focusedIndex === 0}
              selectedIndex={["crystal", "lesson", "rule"].indexOf(type)}
              showDescription={false}
              selectedTextColor="cyan"
              selectedBackgroundColor="transparent"
              focusedBackgroundColor="transparent"
              options={[
                {
                  name: "Crystal",
                  description: "Long-term memory",
                  value: "crystal",
                },
                {
                  name: "Lesson",
                  description: "Learned lesson",
                  value: "lesson",
                },
                { name: "Rule", description: "Strict rule", value: "rule" },
              ]}
              onChange={(_index, option) => {
                if (option?.value) {
                  setType(String(option.value));
                }
              }}
              onSelect={(_index, option) => {
                if (option?.value) {
                  setType(String(option.value));
                }
              }}
            />
          </box>

          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 1 ? "cyan" : "gray"}>Title: </text>
            <TextInput
              value={title}
              onChange={setTitle}
              focus={focusedIndex === 1}
              placeholder="Enter title..."
            />
          </box>

          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 2 ? "cyan" : "gray"}>Text: </text>
            <TextAreaInput
              value={text}
              onChange={setText}
              focus={focusedIndex === 2}
              placeholder="Enter content/observation..."
              onSubmit={handleSubmit}
            />
          </box>

          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 3 ? "cyan" : "gray"}>Tags: </text>
            <TextInput
              value={tags}
              onChange={setTags}
              focus={focusedIndex === 3}
              placeholder="tag1, tag2..."
              onSubmit={handleSubmit}
            />
          </box>
        </box>
        <box marginTop={1} justifyContent="space-between">
          <text fg="gray">[Enter] Submit</text>
          <text fg="gray">[Esc] Cancel</text>
        </box>
      </box>
    </box>
  );
}

// 3. Edit Dialog
function EditDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
}) {
  const [title, setTitle] = useState(state.initialTitle || "");
  const [text, setText] = useState(state.initialText || "");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0 = title, 1 = text

  const handleSubmit = () => {
    onSubmit({
      id: state.entityId,
      title,
      text,
    });
  };

  useKeyboard((key) => {
    if (key.name === "escape") {
      onClose();
      return;
    }
    if (key.name === "enter") {
      handleSubmit();
      return;
    }
    if (key.name === "up") {
      setFocusedIndex(0);
    } else if (key.name === "down") {
      setFocusedIndex(1);
    }
  });

  return (
    <box {...overlayStyle}>
      <box {...modalStyle}>
        <text fg="cyan">Edit Memory</text>
        <box marginTop={1} flexDirection="column">
          <box flexDirection="row">
            <text fg={focusedIndex === 0 ? "cyan" : "gray"}>Title: </text>
            <TextInput
              value={title}
              onChange={setTitle}
              focus={focusedIndex === 0}
              placeholder="Enter title..."
            />
          </box>

          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 1 ? "cyan" : "gray"}>Text: </text>
            <TextAreaInput
              value={text}
              onChange={setText}
              focus={focusedIndex === 1}
              placeholder="Enter content..."
              onSubmit={handleSubmit}
            />
          </box>
        </box>
        {state.error ? (
          <box marginTop={1}>
            <text fg="red">{state.error}</text>
          </box>
        ) : null}
        <box marginTop={1} justifyContent="space-between">
          <text fg="gray">[Enter] Submit</text>
          <text fg="gray">[Esc] Cancel</text>
        </box>
      </box>
    </box>
  );
}

// 4. Rename Dialog
function RenameDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
}) {
  const [name, setName] = useState(state.initialTitle || "");

  useKeyboard((key) => {
    if (key.name === "escape") {
      onClose();
    }
  });

  const handleSubmit = () => {
    onSubmit({
      concept_id: state.entityId,
      canonical_name: name,
    });
  };

  return (
    <box {...overlayStyle}>
      <box {...modalStyle}>
        <text fg="cyan">Rename Concept</text>
        <box marginTop={1} flexDirection="row">
          <text fg="cyan">Name: </text>
          <TextInput
            value={name}
            onChange={setName}
            focus={true}
            placeholder="Enter canonical name..."
            onSubmit={handleSubmit}
          />
        </box>
        {state.error ? (
          <box marginTop={1}>
            <text fg="red">{state.error}</text>
          </box>
        ) : null}
        <box marginTop={1} justifyContent="space-between">
          <text fg="gray">[Enter] Submit</text>
          <text fg="gray">[Esc] Cancel</text>
        </box>
      </box>
    </box>
  );
}

// 5. Merge Dialog
function MergeDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
}) {
  const isConcept = state.entityType === "concept";
  const [targetId, setTargetId] = useState("");
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [evidence, setEvidence] = useState("");
  const [focusedIndex, setFocusedIndex] = useState(0);
  const [localError, setLocalError] = useState("");

  const maxIndex = isConcept ? 1 : 2; // concept: 0=targetId, 1=evidence. crystal: 0=targetId, 1=title, 2=text.

  const handleSubmit = () => {
    const parsedTarget = parseInt(targetId, 10);
    if (!targetId.trim() || isNaN(parsedTarget)) {
      setLocalError("Target ID must be a valid number");
      return;
    }
    setLocalError("");

    if (isConcept) {
      onSubmit({
        source_concept_id: state.entityId,
        target_concept_id: parsedTarget,
        evidence,
      });
    } else {
      onSubmit({
        ids: [state.entityId, parsedTarget],
        title,
        text,
      });
    }
  };

  useKeyboard((key) => {
    if (key.name === "escape") {
      onClose();
      return;
    }
    if (key.name === "enter") {
      handleSubmit();
      return;
    }
    if (key.name === "up") {
      setFocusedIndex((prev) => Math.max(0, prev - 1));
    } else if (key.name === "down") {
      setFocusedIndex((prev) => Math.min(maxIndex, prev + 1));
    }
  });

  return (
    <box {...overlayStyle}>
      <box {...modalStyle}>
        <text fg="cyan">Merge {isConcept ? "Concepts" : "Crystals"}</text>
        <box marginTop={1} flexDirection="column">
          <text>
            Merging source {isConcept ? "Concept" : "Crystal"} ID:{" "}
            {state.entityId}
          </text>
          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 0 ? "cyan" : "gray"}>Target ID: </text>
            <TextInput
              value={targetId}
              onChange={setTargetId}
              focus={focusedIndex === 0}
              placeholder="Enter target ID..."
            />
          </box>

          {isConcept ? (
            <box flexDirection="row" marginTop={1}>
              <text fg={focusedIndex === 1 ? "cyan" : "gray"}>Reason: </text>
              <TextInput
                value={evidence}
                onChange={setEvidence}
                focus={focusedIndex === 1}
                placeholder="Merge evidence/reason..."
                onSubmit={handleSubmit}
              />
            </box>
          ) : (
            <>
              <box flexDirection="row" marginTop={1}>
                <text fg={focusedIndex === 1 ? "cyan" : "gray"}>Title: </text>
                <TextInput
                  value={title}
                  onChange={setTitle}
                  focus={focusedIndex === 1}
                  placeholder="Merged title..."
                />
              </box>
              <box flexDirection="row" marginTop={1}>
                <text fg={focusedIndex === 2 ? "cyan" : "gray"}>Text: </text>
                <TextAreaInput
                  value={text}
                  onChange={setText}
                  focus={focusedIndex === 2}
                  placeholder="Merged content..."
                  onSubmit={handleSubmit}
                />
              </box>
            </>
          )}
        </box>
        {localError || state.error ? (
          <box marginTop={1}>
            <text fg="red">{localError || state.error}</text>
          </box>
        ) : null}
        <box marginTop={1} justifyContent="space-between">
          <text fg="gray">[Enter] Submit</text>
          <text fg="gray">[Esc] Cancel</text>
        </box>
      </box>
    </box>
  );
}

// 6. Split Dialog
function SplitDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
}) {
  const [partOneTitle, setPartOneTitle] = useState("");
  const [partOneText, setPartOneText] = useState("");
  const [partTwoTitle, setPartTwoTitle] = useState("");
  const [partTwoText, setPartTwoText] = useState("");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0=p1 title, 1=p1 text, 2=p2 title, 3=p2 text

  const handleSubmit = () => {
    onSubmit({
      id: state.entityId,
      part_one_title: partOneTitle,
      part_one_text: partOneText,
      part_two_title: partTwoTitle,
      part_two_text: partTwoText,
    });
  };

  useKeyboard((key) => {
    if (key.name === "escape") {
      onClose();
      return;
    }
    if (key.name === "enter") {
      handleSubmit();
      return;
    }
    if (key.name === "up") {
      setFocusedIndex((prev) => Math.max(0, prev - 1));
    } else if (key.name === "down") {
      setFocusedIndex((prev) => Math.min(3, prev + 1));
    }
  });

  return (
    <box {...overlayStyle}>
      <box {...modalStyle}>
        <text fg="cyan">Split Crystal</text>
        <box marginTop={1} flexDirection="column">
          <text>Splitting Crystal ID: {state.entityId}</text>
          <box marginTop={1}>
            <text>Part 1</text>
          </box>
          <box flexDirection="row">
            <text fg={focusedIndex === 0 ? "cyan" : "gray"}>Title: </text>
            <TextInput
              value={partOneTitle}
              onChange={setPartOneTitle}
              focus={focusedIndex === 0}
              placeholder="Part 1 title..."
            />
          </box>
          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 1 ? "cyan" : "gray"}>Text: </text>
            <TextAreaInput
              value={partOneText}
              onChange={setPartOneText}
              focus={focusedIndex === 1}
              placeholder="Part 1 content..."
              onSubmit={handleSubmit}
            />
          </box>

          <box marginTop={1}>
            <text>Part 2</text>
          </box>
          <box flexDirection="row">
            <text fg={focusedIndex === 2 ? "cyan" : "gray"}>Title: </text>
            <TextInput
              value={partTwoTitle}
              onChange={setPartTwoTitle}
              focus={focusedIndex === 2}
              placeholder="Part 2 title..."
            />
          </box>
          <box flexDirection="row" marginTop={1}>
            <text fg={focusedIndex === 3 ? "cyan" : "gray"}>Text: </text>
            <TextAreaInput
              value={partTwoText}
              onChange={setPartTwoText}
              focus={focusedIndex === 3}
              placeholder="Part 2 content..."
              onSubmit={handleSubmit}
            />
          </box>
        </box>
        {state.error ? (
          <box marginTop={1}>
            <text fg="red">{state.error}</text>
          </box>
        ) : null}
        <box marginTop={1} justifyContent="space-between">
          <text fg="gray">[Enter] Submit</text>
          <text fg="gray">[Esc] Cancel</text>
        </box>
      </box>
    </box>
  );
}
