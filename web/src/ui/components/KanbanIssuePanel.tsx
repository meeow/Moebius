// From BloopAI/vibe-kanban packages/ui/src/components/KanbanIssuePanel.tsx (Apache-2.0), changed: the panel is a
// read-only inspector, not the form for creating and editing an issue (the title, description editor, tags, assignees
// and attachments are gone); it keeps the frame — the header with the number, the copy-link and close buttons, Escape
// to close, the scrolling body — and takes its sections as children; it is a drawer beside the board, so it is as tall
// as the box that holds it, separating its sections with space instead of rules, labelling its controls, and fading
// the edges its body cuts off. Where it stands the full height of the window it is flush to its three edges and part
// of the page rather than a sheet over it, so it has no radius, no shadow, and one hairline where it meets the board.
import { LinkIcon, XIcon } from "@phosphor-icons/react";
import { type KeyboardEvent, type ReactNode, useId } from "react";
import { cn } from "../lib/cn";
import { useScrollFades } from "../page/ScrollFades";
import { Button } from "./Button";
import { IconButton } from "./IconButton";

export interface KanbanIssuePanelProps {
  /** The card's number as it is written, such as `#12`. */
  displayId: string;
  /** Where the card's issue is, when the app knows: the number is then a link. */
  issueUrl?: string | null;
  title: string;
  /** What sits beside the number, such as the card's status. */
  status?: ReactNode;
  onClose: () => void;
  /** Copies the address of the card; the button shows only with it. */
  onCopyLink?: () => void;
  className?: string;
  /** The sections, each in a `KanbanIssueSection`. */
  children: ReactNode;
}

export function KanbanIssuePanel({
  displayId,
  issueUrl,
  title,
  status,
  onClose,
  onCopyLink,
  className,
  children,
}: KanbanIssuePanelProps) {
  // A card's sections are read from the top and the dock scrolls the whole height of the board, so both edges
  // are usually cut mid-sentence.
  const fades = useScrollFades<HTMLDivElement>();

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    const target = e.target as HTMLElement;
    // Escape belongs to the topmost layer. A sheet the inspector itself opened — the diff of a change — is inside
    // this section in the tree that opened it, even though it draws over the whole page, so its own Escape is
    // stopped here rather than closing the card behind it as well.
    if (target.closest('[role="dialog"]')) return;
    const isEditable =
      target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable;
    if (isEditable) {
      target.blur();
      e.stopPropagation();
    } else {
      onClose();
    }
  };

  const number = (
    <span className="flex min-w-0 items-center gap-1 text-sm text-secondary">
      <span>Card</span>
      <span className="font-mono text-text">{displayId}</span>
    </span>
  );

  return (
    <section
      aria-label="Card inspector"
      className={cn(
        "flex h-full max-h-screen min-h-0 flex-col overflow-hidden rounded-sheet bg-surface shadow-card outline-none lg:rounded-none lg:border-l lg:border-hairline lg:shadow-none",
        className,
      )}
      onKeyDown={handleKeyDown}
      tabIndex={-1}
    >
      {/* Head: where the card is, what it is, and the two ways out of the sheet. */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 px-4 pt-4">
        <div className="flex min-w-0 items-center gap-2">
          {issueUrl ? (
            <a
              href={issueUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="-ml-2 inline-flex min-h-10 items-center rounded-control px-2 transition-colors duration-(--motion-routine) ease-out hover:bg-hover"
            >
              {number}
            </a>
          ) : (
            <span className="-ml-2 inline-flex min-h-10 items-center rounded-control px-2">
              {number}
            </span>
          )}
          {onCopyLink && (
            <Button variant="ghost" size="sm" onClick={onCopyLink}>
              <LinkIcon className="size-4" weight="bold" aria-hidden="true" />
              Copy link
            </Button>
          )}
        </div>
        <div className="flex items-center gap-2">
          {status}
          <IconButton icon={XIcon} aria-label="Close the inspector" onClick={onClose} />
        </div>
      </div>

      {/* Scrollable content: the title wraps, however long, and the edges with more behind them fade out */}
      <div
        ref={fades.box}
        onScroll={fades.measure}
        style={fades.style}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <h2 className="m-0 px-4 pt-2 pb-4 text-lg font-semibold tracking-tight text-text [overflow-wrap:anywhere]">
          {title}
        </h2>
        {children}
      </div>
    </section>
  );
}

/** A section of the panel: a heading and its content, set apart by space rather than by a rule. */
export function KanbanIssueSection({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  // The heading says the title once; the section is named by pointing at it rather than by repeating the words.
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className={cn("px-4 pt-4 pb-6", className)}>
      <h3 id={headingId} className="m-0 mb-3 text-sm font-semibold text-secondary">
        {title}
      </h3>
      {children}
    </section>
  );
}
