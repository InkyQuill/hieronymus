import { useState } from "react";

export type FieldFocus = {
  focusedIndex: number;
  moveUp: () => void;
  moveDown: () => void;
  setFocusedIndex: (index: number) => void;
};

export function useFieldFocus(fieldCount: number): FieldFocus {
  const [focusedIndex, setFocusedIndex] = useState(0);

  const moveUp = () => {
    setFocusedIndex((current) => Math.max(0, current - 1));
  };

  const moveDown = () => {
    setFocusedIndex((current) =>
      Math.min(Math.max(fieldCount - 1, 0), current + 1),
    );
  };

  return { focusedIndex, moveUp, moveDown, setFocusedIndex };
}
