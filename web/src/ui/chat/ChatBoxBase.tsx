// From BloopAI/vibe-kanban packages/ui/src/components/ChatBoxBase.tsx (Apache-2.0), changed: no i18n, drag and drop,
// visual variants or header; the box is a card of the surface colour with a soft shadow rather than a bordered panel,
// or one row inside a sheet (`inline`),
// and a refusal is said by the shared Notice above the field. Whether the agent works is said once, by the
// conversation and by the page's hint, so the box itself stays out of it.
import type { ReactNode } from "react";
import { cn } from "../lib/cn";
import { Notice } from "../page/Notice";

interface ChatBoxBaseProps {
  /** The field to write in. */
  editor: ReactNode;
  /** A line above the field, such as the last thing that went wrong. */
  error?: string | null;
  /** The choices on the left of the footer: the agent, the model, the reasoning level. */
  pills?: ReactNode;
  /** The pictures held with the message, in a row above the field. */
  pictures?: ReactNode;
  /** The buttons on the right of the footer. */
  footerRight: ReactNode;
  className?: string;
  /**
   * One row instead of two, and no card of its own, for a composer already inside a sheet: the field with its buttons
   * beside it. The sheet is the layer that is raised; a second shadow here is what makes a panel look like two panels.
   */
  inline?: boolean;
}

/** The box a message is written in: the field on top, and under it the choices on the left and the buttons on the right. */
export function ChatBoxBase({
  editor,
  error,
  pills,
  pictures,
  footerRight,
  className,
  inline = false,
}: ChatBoxBaseProps) {
  return (
    <div
      className={cn(
        "relative flex w-full gap-3",
        inline ? "items-end" : "flex-col rounded-card bg-surface p-3 shadow-card",
        className,
      )}
    >
      {error && <Notice tone="error">{error}</Notice>}
      {pictures}
      {editor}
      {inline ? (
        <div className="flex shrink-0 items-center gap-2">
          {pills}
          {footerRight}
        </div>
      ) : (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex min-w-0 flex-wrap items-center gap-2">{pills}</div>
          <div className="flex shrink-0 items-center gap-3">{footerRight}</div>
        </div>
      )}
    </div>
  );
}
