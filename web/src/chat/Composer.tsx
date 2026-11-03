/**
 * The island a message is written in: an auto-growing field (Enter sends, Shift+Enter makes a new line, and an Enter
 * that confirms an input-method candidate does neither), the pills on its left and the Send button on its right. The
 * pills, the notice and what Send does are the page's; this holds the field. With `onHold` it takes pictures too —
 * pasted, dropped or picked — which a page sends with the message when its model looks at images.
 */
import { PaperclipIcon, PaperPlaneRightIcon, XIcon } from "@phosphor-icons/react";
import type { ClipboardEvent, DragEvent } from "react";
import { type ReactNode, useEffect, useLayoutEffect, useRef } from "react";
import { ChatBoxBase } from "../ui/chat/ChatBoxBase";
import { Button } from "../ui/components/Button";
import { isImeConfirmation } from "../ui/lib/imeComposition";
import { type HeldPicture, pictureFiles } from "./pictures";

/** The most rows the field grows to before it scrolls. */
const MAX_ROWS = 8;

interface ComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  /** Whether Send can be pressed (there is something to send, and the chat can take it). */
  canSend: boolean;
  placeholder?: string;
  pills?: ReactNode;
  /** The last thing that went wrong, as a short line above the field. */
  error?: string | null;
  /** Whether the agent works: the island lights up. */
  running?: boolean;
  /** Something between the pills and the buttons on the right, such as a note. */
  extra?: ReactNode;
  autoFocus?: boolean;
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
  placeholder = "Write to the agent",
  pills,
  error,
  running,
  extra,
  autoFocus,
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

  // Grows the field with its text, up to MAX_ROWS lines.
  // biome-ignore lint/correctness/useExhaustiveDependencies: it runs when the text changes
  useLayoutEffect(() => {
    const element = field.current;
    if (!element) return;
    element.style.height = "auto";
    const line = Number.parseFloat(getComputedStyle(element).lineHeight) || 20;
    element.style.height = `${Math.min(element.scrollHeight, line * MAX_ROWS)}px`;
  }, [value]);

  /** Takes the pictures a paste or a drop carries, and stops the browser doing its own thing with them. */
  function take(
    event: ClipboardEvent<HTMLElement> | DragEvent<HTMLElement>,
    data: DataTransfer | null,
  ): void {
    if (onHold === undefined) return;
    const files = pictureFiles(data);
    if (files.length === 0) return;
    event.preventDefault();
    onHold(files);
  }

  return (
    // The island is a fieldset so that pasting or dropping a picture lands on an element that groups controls; the
    // clip button, whose field it opens, is the way in for a keyboard and a screen reader.
    <fieldset
      aria-label="Message and its pictures"
      className="min-w-0 border-0 p-0"
      onPaste={(event) => take(event, event.clipboardData)}
      onDragOver={(event) => {
        if (onHold !== undefined && (event.dataTransfer?.types.includes("Files") ?? false)) {
          event.preventDefault();
        }
      }}
      onDrop={(event) => take(event, event.dataTransfer)}
    >
      <ChatBoxBase
        error={error}
        isRunning={running}
        pills={pills}
        pictures={
          pictures.length === 0 ? undefined : (
            <ul
              aria-label="Pictures with this message"
              className="flex flex-wrap gap-half rounded-md bg-card px-base py-base"
            >
              {pictures.map((picture) => (
                <li key={picture.key} className="relative">
                  <img
                    src={picture.url}
                    alt={picture.name ?? "A picture with this message"}
                    title={`${picture.name ?? "A picture"} (${picture.media_type})`}
                    className="size-16 rounded border border-line object-cover"
                  />
                  {onUnhold !== undefined && (
                    <button
                      type="button"
                      aria-label={`Take ${picture.name ?? "the picture"} off the message`}
                      title="Take it off the message"
                      onClick={() => onUnhold(picture)}
                      className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full border border-line-strong bg-panel text-low hover:bg-card-hover hover:text-high"
                    >
                      <XIcon className="size-2.5" weight="bold" />
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )
        }
        editor={
          <textarea
            ref={field}
            value={value}
            rows={1}
            aria-label="Message"
            placeholder={placeholder}
            onChange={(event) => onChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.shiftKey || isImeConfirmation(event.nativeEvent)) {
                return;
              }
              event.preventDefault();
              if (canSend) onSend();
            }}
            className="max-h-60 w-full resize-none bg-transparent py-half text-sm text-high outline-none placeholder:text-low"
          />
        }
        footerRight={
          <>
            {extra}
            {onHold !== undefined && (
              <>
                <Button
                  variant="ghost"
                  size="xs"
                  aria-label="Attach a picture"
                  title={
                    pictures.length === 0
                      ? "Attach a picture, or paste or drop one here"
                      : `Attach one of the ${6 - pictures.length} pictures a message carries`
                  }
                  onClick={() => picker.current?.click()}
                >
                  <PaperclipIcon className="size-icon-xs" />
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
            <Button size="sm" onClick={onSend} disabled={!canSend}>
              <PaperPlaneRightIcon className="size-icon-xs" weight="fill" />
              Send
            </Button>
          </>
        }
      />
    </fieldset>
  );
}
