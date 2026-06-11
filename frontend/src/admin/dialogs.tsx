import React, { useState } from "react";
import { Box, Text, useInput, useStdin } from "ink";
import { TextInput } from "../ui/TextInput.js";

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
  const { stdin, isRawModeSupported } = useStdin();
  const canUseInkInput = Boolean(
    isRawModeSupported &&
      typeof stdin.ref === "function" &&
      typeof stdin.unref === "function",
  );

  if (state.kind === "none") {
    return null;
  }

  // Common styles
  const overlayStyle: any = {
    position: "absolute",
    width: "100%",
    height: "100%",
    alignItems: "center",
    justifyContent: "center",
  };

  const modalStyle: any = {
    borderStyle: "double",
    borderColor: "cyan",
    flexDirection: "column",
    padding: 1,
    minWidth: 60,
  };

  if (state.kind === "delete") {
    return (
      <DeleteDialog
        state={state}
        onClose={onClose}
        onSubmit={onSubmit}
        overlayStyle={overlayStyle}
        modalStyle={modalStyle}
        canUseInkInput={canUseInkInput}
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
        canUseInkInput={canUseInkInput}
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
        canUseInkInput={canUseInkInput}
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
        canUseInkInput={canUseInkInput}
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
        canUseInkInput={canUseInkInput}
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
        canUseInkInput={canUseInkInput}
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
  canUseInkInput,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
  canUseInkInput: boolean;
}) {
  useInput(
    (_input, key) => {
      if (key.escape || _input === "n" || _input === "N") {
        onClose();
      } else if (key.return || _input === "y" || _input === "Y") {
        if (state.entityType === "concept") {
          onSubmit({ concept_id: state.entityId, confirmed: true });
        } else if (state.entityType === "memory") {
          onSubmit({ memory_id: state.entityId, confirmed: true });
        } else {
          onSubmit({ id: state.entityId, confirmed: true });
        }
      }
    },
    { isActive: canUseInkInput },
  );

  return (
    <Box {...overlayStyle}>
      <Box {...modalStyle} borderColor="red">
        <Text bold color="red">
          Confirm Deletion
        </Text>
        <Box marginTop={1}>
          <Text>
            Are you sure you want to delete this {state.entityType || "item"}?
          </Text>
        </Box>
        <Box marginTop={1} flexDirection="row">
          <Text color="gray">ID: </Text>
          <Text>{state.entityId}</Text>
        </Box>
        {state.error ? (
          <Box marginTop={1}>
            <Text color="red">{state.error}</Text>
          </Box>
        ) : null}
        <Box marginTop={1} justifyContent="space-between">
          <Text dimColor>[Y] Yes, Delete</Text>
          <Text dimColor>[Esc/N] Cancel</Text>
        </Box>
      </Box>
    </Box>
  );
}

// 2. Add Dialog
function AddDialog({
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
  canUseInkInput,
}: {
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
  canUseInkInput: boolean;
}) {
  const [type, setType] = useState<"crystal" | "lesson" | "rule" | string>("crystal");
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [tags, setTags] = useState("");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0 = type, 1 = title, 2 = text, 3 = tags

  useInput(
    (input, key) => {
      if (key.escape) {
        onClose();
        return;
      }
      if (key.upArrow) {
        setFocusedIndex((prev) => Math.max(0, prev - 1));
      } else if (key.downArrow) {
        setFocusedIndex((prev) => Math.min(3, prev + 1));
      } else if (focusedIndex === 0) {
        if (key.leftArrow || key.rightArrow || input === " ") {
          setType((prev) => {
            if (prev === "crystal") return "lesson";
            if (prev === "lesson") return "rule";
            return "crystal";
          });
        }
      }
    },
    { isActive: canUseInkInput },
  );

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

  return (
    <Box {...overlayStyle}>
      <Box {...modalStyle}>
        <Text bold color="cyan">
          Add New Crystal / Lesson / Rule
        </Text>
        <Box marginTop={1} flexDirection="column">
          <Box flexDirection="row">
            <Text color={focusedIndex === 0 ? "cyan" : "gray"}>Type: </Text>
            <Text
              color={type === "crystal" ? "cyan" : undefined}
              bold={type === "crystal"}
            >
              [Crystal]{" "}
            </Text>
            <Text
              color={type === "lesson" ? "cyan" : undefined}
              bold={type === "lesson"}
            >
              [Lesson]{" "}
            </Text>
            <Text color={type === "rule" ? "cyan" : undefined} bold={type === "rule"}>
              [Rule]
            </Text>
          </Box>

          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 1 ? "cyan" : "gray"}>Title: </Text>
            <TextInput
              value={title}
              onChange={setTitle}
              focus={focusedIndex === 1}
              placeholder="Enter title..."
            />
          </Box>

          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 2 ? "cyan" : "gray"}>Text: </Text>
            <TextInput
              value={text}
              onChange={setText}
              focus={focusedIndex === 2}
              placeholder="Enter content/observation..."
            />
          </Box>

          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 3 ? "cyan" : "gray"}>Tags: </Text>
            <TextInput
              value={tags}
              onChange={setTags}
              focus={focusedIndex === 3}
              placeholder="tag1, tag2..."
              onSubmit={handleSubmit}
            />
          </Box>
        </Box>
        <Box marginTop={1} justifyContent="space-between">
          <Text dimColor>[Enter] Submit</Text>
          <Text dimColor>[Esc] Cancel</Text>
        </Box>
      </Box>
    </Box>
  );
}

// 3. Edit Dialog
function EditDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
  canUseInkInput,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
  canUseInkInput: boolean;
}) {
  const [title, setTitle] = useState(state.initialTitle || "");
  const [text, setText] = useState(state.initialText || "");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0 = title, 1 = text

  useInput(
    (_input, key) => {
      if (key.escape) {
        onClose();
        return;
      }
      if (key.upArrow) {
        setFocusedIndex(0);
      } else if (key.downArrow) {
        setFocusedIndex(1);
      }
    },
    { isActive: canUseInkInput },
  );

  const handleSubmit = () => {
    onSubmit({
      id: state.entityId,
      title,
      text,
    });
  };

  return (
    <Box {...overlayStyle}>
      <Box {...modalStyle}>
        <Text bold color="cyan">
          Edit Memory
        </Text>
        <Box marginTop={1} flexDirection="column">
          <Box flexDirection="row">
            <Text color={focusedIndex === 0 ? "cyan" : "gray"}>Title: </Text>
            <TextInput
              value={title}
              onChange={setTitle}
              focus={focusedIndex === 0}
              placeholder="Enter title..."
            />
          </Box>

          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 1 ? "cyan" : "gray"}>Text: </Text>
            <TextInput
              value={text}
              onChange={setText}
              focus={focusedIndex === 1}
              placeholder="Enter content..."
              onSubmit={handleSubmit}
            />
          </Box>
        </Box>
        {state.error ? (
          <Box marginTop={1}>
            <Text color="red">{state.error}</Text>
          </Box>
        ) : null}
        <Box marginTop={1} justifyContent="space-between">
          <Text dimColor>[Enter] Submit</Text>
          <Text dimColor>[Esc] Cancel</Text>
        </Box>
      </Box>
    </Box>
  );
}

// 4. Rename Dialog
function RenameDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
  canUseInkInput,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
  canUseInkInput: boolean;
}) {
  const [name, setName] = useState(state.initialTitle || "");

  useInput(
    (_input, key) => {
      if (key.escape) {
        onClose();
      }
    },
    { isActive: canUseInkInput },
  );

  const handleSubmit = () => {
    onSubmit({
      concept_id: state.entityId,
      canonical_name: name,
    });
  };

  return (
    <Box {...overlayStyle}>
      <Box {...modalStyle}>
        <Text bold color="cyan">
          Rename Concept
        </Text>
        <Box marginTop={1} flexDirection="row">
          <Text color="cyan">Name: </Text>
          <TextInput
            value={name}
            onChange={setName}
            focus={true}
            placeholder="Enter canonical name..."
            onSubmit={handleSubmit}
          />
        </Box>
        {state.error ? (
          <Box marginTop={1}>
            <Text color="red">{state.error}</Text>
          </Box>
        ) : null}
        <Box marginTop={1} justifyContent="space-between">
          <Text dimColor>[Enter] Submit</Text>
          <Text dimColor>[Esc] Cancel</Text>
        </Box>
      </Box>
    </Box>
  );
}

// 5. Merge Dialog
function MergeDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
  canUseInkInput,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
  canUseInkInput: boolean;
}) {
  const isConcept = state.entityType === "concept";
  const [targetId, setTargetId] = useState("");
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [evidence, setEvidence] = useState("");
  const [focusedIndex, setFocusedIndex] = useState(0);

  const maxIndex = isConcept ? 1 : 2; // concept: 0=targetId, 1=evidence. crystal: 0=targetId, 1=title, 2=text.

  useInput(
    (_input, key) => {
      if (key.escape) {
        onClose();
        return;
      }
      if (key.upArrow) {
        setFocusedIndex((prev) => Math.max(0, prev - 1));
      } else if (key.downArrow) {
        setFocusedIndex((prev) => Math.min(maxIndex, prev + 1));
      }
    },
    { isActive: canUseInkInput },
  );

  const handleSubmit = () => {
    if (isConcept) {
      onSubmit({
        source_concept_id: state.entityId,
        target_concept_id: parseInt(targetId, 10),
        evidence,
      });
    } else {
      onSubmit({
        ids: [state.entityId, parseInt(targetId, 10)].filter(
          (id) => !isNaN(id as number),
        ),
        title,
        text,
      });
    }
  };

  return (
    <Box {...overlayStyle}>
      <Box {...modalStyle}>
        <Text bold color="cyan">
          Merge {isConcept ? "Concepts" : "Crystals"}
        </Text>
        <Box marginTop={1} flexDirection="column">
          <Text>
            Merging source {isConcept ? "Concept" : "Crystal"} ID:{" "}
            {state.entityId}
          </Text>
          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 0 ? "cyan" : "gray"}>Target ID: </Text>
            <TextInput
              value={targetId}
              onChange={setTargetId}
              focus={focusedIndex === 0}
              placeholder="Enter target ID..."
            />
          </Box>

          {isConcept ? (
            <Box flexDirection="row" marginTop={1}>
              <Text color={focusedIndex === 1 ? "cyan" : "gray"}>Reason: </Text>
              <TextInput
                value={evidence}
                onChange={setEvidence}
                focus={focusedIndex === 1}
                placeholder="Merge evidence/reason..."
                onSubmit={handleSubmit}
              />
            </Box>
          ) : (
            <>
              <Box flexDirection="row" marginTop={1}>
                <Text color={focusedIndex === 1 ? "cyan" : "gray"}>Title: </Text>
                <TextInput
                  value={title}
                  onChange={setTitle}
                  focus={focusedIndex === 1}
                  placeholder="Merged title..."
                />
              </Box>
              <Box flexDirection="row" marginTop={1}>
                <Text color={focusedIndex === 2 ? "cyan" : "gray"}>Text: </Text>
                <TextInput
                  value={text}
                  onChange={setText}
                  focus={focusedIndex === 2}
                  placeholder="Merged content..."
                  onSubmit={handleSubmit}
                />
              </Box>
            </>
          )}
        </Box>
        {state.error ? (
          <Box marginTop={1}>
            <Text color="red">{state.error}</Text>
          </Box>
        ) : null}
        <Box marginTop={1} justifyContent="space-between">
          <Text dimColor>[Enter] Submit</Text>
          <Text dimColor>[Esc] Cancel</Text>
        </Box>
      </Box>
    </Box>
  );
}

// 6. Split Dialog
function SplitDialog({
  state,
  onClose,
  onSubmit,
  overlayStyle,
  modalStyle,
  canUseInkInput,
}: {
  state: DialogState;
  onClose: () => void;
  onSubmit: (params: Record<string, any>) => void;
  overlayStyle: any;
  modalStyle: any;
  canUseInkInput: boolean;
}) {
  const [partOneTitle, setPartOneTitle] = useState("");
  const [partOneText, setPartOneText] = useState("");
  const [partTwoTitle, setPartTwoTitle] = useState("");
  const [partTwoText, setPartTwoText] = useState("");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0=p1 title, 1=p1 text, 2=p2 title, 3=p2 text

  useInput(
    (_input, key) => {
      if (key.escape) {
        onClose();
        return;
      }
      if (key.upArrow) {
        setFocusedIndex((prev) => Math.max(0, prev - 1));
      } else if (key.downArrow) {
        setFocusedIndex((prev) => Math.min(3, prev + 1));
      }
    },
    { isActive: canUseInkInput },
  );

  const handleSubmit = () => {
    onSubmit({
      id: state.entityId,
      part_one_title: partOneTitle,
      part_one_text: partOneText,
      part_two_title: partTwoTitle,
      part_two_text: partTwoText,
    });
  };

  return (
    <Box {...overlayStyle}>
      <Box {...modalStyle}>
        <Text bold color="cyan">
          Split Crystal
        </Text>
        <Box marginTop={1} flexDirection="column">
          <Text>Splitting Crystal ID: {state.entityId}</Text>
          <Box marginTop={1}>
            <Text bold>
              Part 1
            </Text>
          </Box>
          <Box flexDirection="row">
            <Text color={focusedIndex === 0 ? "cyan" : "gray"}>Title: </Text>
            <TextInput
              value={partOneTitle}
              onChange={setPartOneTitle}
              focus={focusedIndex === 0}
              placeholder="Part 1 title..."
            />
          </Box>
          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 1 ? "cyan" : "gray"}>Text: </Text>
            <TextInput
              value={partOneText}
              onChange={setPartOneText}
              focus={focusedIndex === 1}
              placeholder="Part 1 content..."
            />
          </Box>

          <Box marginTop={1}>
            <Text bold>
              Part 2
            </Text>
          </Box>
          <Box flexDirection="row">
            <Text color={focusedIndex === 2 ? "cyan" : "gray"}>Title: </Text>
            <TextInput
              value={partTwoTitle}
              onChange={setPartTwoTitle}
              focus={focusedIndex === 2}
              placeholder="Part 2 title..."
            />
          </Box>
          <Box flexDirection="row" marginTop={1}>
            <Text color={focusedIndex === 3 ? "cyan" : "gray"}>Text: </Text>
            <TextInput
              value={partTwoText}
              onChange={setPartTwoText}
              focus={focusedIndex === 3}
              placeholder="Part 2 content..."
              onSubmit={handleSubmit}
            />
          </Box>
        </Box>
        {state.error ? (
          <Box marginTop={1}>
            <Text color="red">{state.error}</Text>
          </Box>
        ) : null}
        <Box marginTop={1} justifyContent="space-between">
          <Text dimColor>[Enter] Submit</Text>
          <Text dimColor>[Esc] Cancel</Text>
        </Box>
      </Box>
    </Box>
  );
}
