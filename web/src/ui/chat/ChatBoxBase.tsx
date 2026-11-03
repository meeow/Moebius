// From BloopAI/vibe-kanban packages/ui/src/components/ChatBoxBase.tsx (Apache-2.0), changed: no i18n, drag and drop, visual variants or header; the island is the composer of the desktop app's CH3-D layout (the field on top, the pills on the left and the buttons on the right, the accent's line around it); `isRunning` draws the line in the accent's strong colour.
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

interface ChatBoxBaseProps {
  /** The field to write in. */
  editor: ReactNode;
  /** A line above the field, such as the last thing that went wrong. */
  error?: string | null;
  /** The pills on the left of the footer. */
  pills?: ReactNode;
  /** The pictures held with the message, in a row above the field. */
  pictures?: ReactNode;
  /** The buttons on the right of the footer. */
  footerRight: ReactNode;
  /** Whether the agent works (the line around the box lights up). */
  isRunning?: boolean;
  className?: string;
}

/** The box a message is written in: the field on top, and under it the pills on the left and the buttons on the right. */
export function ChatBoxBase({
  editor,
  error,
  pills,
  pictures,
  footerRight,
  isRunning,
  className,
}: ChatBoxBaseProps) {
  return (
    <div
      className={cn(
        "relative flex w-full flex-col rounded-xl border bg-panel shadow-md",
        isRunning ? "border-accent" : "border-accent-border",
        className,
      )}
      data-running={isRunning ? "true" : undefined}
    >
      {error && (
        <div role="alert" className="rounded-t-xl border-b bg-error/10 px-double py-base">
          <p className="text-sm text-error">{error}</p>
        </div>
      )}
      <div className="flex flex-col gap-plusfifty px-plusfifty py-base">
        {pictures}
        {editor}
        <div className="flex flex-wrap items-end justify-between gap-base">
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-half">{pills}</div>
          <div className="flex shrink-0 gap-base">{footerRight}</div>
        </div>
      </div>
    </div>
  );
}
