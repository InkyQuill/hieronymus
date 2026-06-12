import React, { useState, useEffect } from "react";
import spinners, { type BrailleSpinnerName } from "unicode-animations";

type Props = {
  name?: BrailleSpinnerName;
  label?: string;
  fg?: string;
};

export function Spinner({ name = "braille", label = "", fg }: Props) {
  const [frame, setFrame] = useState(0);
  const s = spinners[name] || spinners.braille;

  useEffect(() => {
    if (process.env.NODE_ENV === "test") {
      return;
    }
    const timer = setInterval(() => {
      setFrame((f) => (f + 1) % s.frames.length);
    }, s.interval);
    return () => clearInterval(timer);
  }, [s]);

  return (
    <text fg={fg}>
      {s.frames[frame]}
      {label ? ` ${label}` : ""}
    </text>
  );
}
