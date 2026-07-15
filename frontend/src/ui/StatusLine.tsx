import React, { useEffect, useState } from "react";
import { useTimeline } from "@opentui/react";
import { Spinner } from "./Spinner.js";
import { theme } from "./theme.js";

type Props = {
  message: string;
  error?: boolean;
  busy?: boolean;
};

export function StatusLine({ message, error = false, busy = false }: Props) {
  const [pulse, setPulse] = useState(0);
  const timeline = useTimeline({
    duration: 900,
    loop: true,
    autoplay: true,
  });

  useEffect(() => {
    if (!busy) {
      setPulse(0);
      timeline.pause();
      return;
    }
    if (process.env.NODE_ENV === "test") {
      setPulse(1);
      timeline.pause();
      return;
    }
    timeline.once(
      { value: 0 },
      {
        value: 1,
        duration: 900,
        ease: "inOutQuad",
        onUpdate: (animation) => {
          setPulse(animation.targets[0].value as number);
        },
      },
    );
    timeline.restart();
  }, [busy, timeline]);

  const fg = error
    ? theme.statusError
    : busy && pulse > 0.5
      ? theme.accentPrimary
      : theme.statusSuccess;
  return (
    <box flexDirection="row" marginTop={1}>
      {busy && (
        <box marginRight={1}>
          <Spinner />
        </box>
      )}
      <text fg={fg}>{message}</text>
    </box>
  );
}
