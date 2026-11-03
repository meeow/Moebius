/**
 * The island a message is written in: a field that grows with the text (Enter sends, Shift+Enter makes a new line, and
 * an Enter that confirms an input-method candidate does neither), the choices on its left and Send on its right. The
 * choices, the notice and what Send does are the page's; this holds the field, at the size the rest of the chat is read
 * at. Send is only ever pressable when there is something to send; when it is not, the reason waits in its tooltip.
 * With `onHold` the field takes pictures too — pasted, dropped or picked — which the page sends with the message when
 * its model looks at them.
 */
import { PaperclipIcon, PaperPlaneRightIcon, XIcon } from "@phosphor-icons/react";
import type { ClipboardEvent, DragEvent, ReactNode } from "react";
import { useEffect, useLayoutEffect, useRef } from "react";
import { ChatBoxBase } from "../ui/chat/ChatBoxBase";
import { Button } from "../ui/components/Button";
import { isImeConfirmation } from "../ui/lib/imeComposition";
import { TextArea } from "../ui/page/Field";
import { type HeldPicture, MAX_IMAGES, pictureFiles } from "./pictures";

/** The most rows the field grows to before it scrolls. */
const MAX_ROWS = 8;

interface ComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  /** Whether Send can be pressed (there is something to send, and the chat can take it). */
  canSend: boolean;
  /** Why Send cannot be pressed, for the person to read when they ask it. */
  sendHint?: string;
  /**
   * The word on the button. It is "Send" while the chat can be written to, and the page changes it when the button
   * does something else — "Continue" for a chat that has ended, whose message starts a new chat told what was said.
   */
  sendLabel?: string;
  /** What the button is about to do, for the person to read when they ask it. */
  sendTitle?: string;
  placeholder?: string;
  pills?: ReactNode;
  /** The last thing that went wrong, as a short line above the field. */
  error?: string | null;
  autoFocus?: boolean;
  /**
   * One row, with Send as an icon beside the field, for the narrow floating chat. The field is the thing being read
   * and written; the word on the button repeats the placeholder above it.
   */
  compact?: boolean;
  /** The pictures held with the message, shown above the field. */
  pictures?: HeldPicture[];
  /** Files the person pasted, dropped or picked. Without it the composer takes no pictures. */
  onHold?: (files: File[]) => void;
  /** Takes one picture back off the message. */
  onUnhold?: (picture: HeldPicture) => void;
}

export function Composer({
  value,
  onChange,
  onSend,
  canSend,
  sendHint,
  sendLabel = "Send",
  sendTitle,
  placeholder = "Write to the agent",
  pills,
  error,
  autoFocus,
  compact = false,
  pictures = [],
  onHold,
  onUnhold,
}: ComposerProps) {
  const field = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);

  // Focuses the field when the composer appears, where the page asks for it.
  useEffect(() => {
    if (autoFocus) field.current?.focus();
  }, [autoFocus]);

  // Grows the field with its text, up to MAX_ROWS lines. The shared field is only as tall as its content wants to be;
  // this is the cap that keeps a pasted wall of text from pushing the rest of the chat away.
  // biome-ignore lint/correctness/useExhaustiveDependencies: it runs when the text changes
  useLayoutEffect(() => {
    const element = field.current;
    if (!element) return;
    element.style.height = "auto";
    const line = Number.parseFloat(getComputedStyle(element).lineHeight) || 24;
    element.style.height = `${Math.min(element.scrollHeight, line * MAX_ROWS)}px`;
  }, [value]);

  /** The pictures of a paste, a drop or a pick, or nothing when the page takes none. */
  function take(event: ClipboardEvent | DragEvent, transfer: DataTransfer | null) {
    const files = pictureFiles(transfer);
    if (files.length === 0) return;
    // A picture was carried, so what the browser meant by the paste or the drop (put its name in the field, open it)
    // is not what the person meant.
    event.preventDefault();
    onHold?.(files);
  }

  return (
    <ChatBoxBase
      inline={compact}
      error={error}
      pills={pills}
      pictures={
        pictures.length === 0 ? undefined : (
          <ul
            aria-label="Pictures with this message"
            className="flex flex-wrap gap-2 rounded-control bg-sunken p-2"
          >
            {pictures.map((picture) => (
              <li key={picture.key} className="relative">
                <img
                  src={picture.url}
                  alt={picture.name ?? "A picture with this message"}
                  title={`${picture.name ?? "A picture"} (${picture.media_type})`}
                  className="size-16 rounded-control border border-hairline object-cover"
                />
                {onUnhold !== undefined && (
                  <button
                    type="button"
                    aria-label={`Take ${picture.name ?? "the picture"} off the message`}
                    title="Take it off the message"
                    onClick={() => onUnhold(picture)}
                    className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full border border-hairline bg-surface text-secondary hover:bg-hover hover:text-text"
                  >
                    <XIcon aria-hidden="true" className="size-2" weight="bold" />
                  </button>
                )}
              </li>
            ))}
          </ul>
        )
      }
      editor={
        <TextArea
          ref={field}
          value={value}
          rows={1}
          aria-label="Message"
          placeholder={placeholder}
          onPaste={(event) => take(event, event.clipboardData)}
          onDragOver={(event) => {
            if (event.dataTransfer.types.includes("Files")) event.preventDefault();
          }}
          onDrop={(event) => take(event, event.dataTransfer)}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Enter" || event.shiftKey || isImeConfirmation(event.nativeEvent)) {
              return;
            }
            event.preventDefault();
            if (canSend) onSend();
          }}
          className="max-h-60 min-h-10 resize-none"
        />
      }
      footerRight={
        <>
          {onHold !== undefined && (
            <>
              <Button
                variant="ghost"
                size={compact ? "icon" : "sm"}
                aria-label="Attach a picture"
                title={
                  pictures.length === 0
                    ? "Attach a picture, or paste or drop one here"
                    : `Attach one of the ${MAX_IMAGES - pictures.length} pictures a message carries`
                }
                onClick={() => picker.current?.click()}
              >
                <PaperclipIcon aria-hidden="true" className="size-4" />
              </Button>
              <input
                ref={picker}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp"
                multiple
                hidden
                onChange={(event) => {
                  const chosen = Array.from(event.target.files ?? []);
                  // The same file chosen twice is still a picture worth sending, so the field is emptied.
                  event.target.value = "";
                  if (chosen.length > 0) onHold(chosen);
                }}
              />
            </>
          )}
          <Button
            size={compact ? "icon" : "md"}
            onClick={onSend}
            disabled={!canSend}
            aria-label={compact ? sendLabel : undefined}
            title={canSend ? (compact ? sendLabel : sendTitle) : sendHint}
          >
            <PaperPlaneRightIcon aria-hidden="true" className="size-4" weight="fill" />
            {compact ? null : sendLabel}
          </Button>
        </>
      }
    />
  );
}
