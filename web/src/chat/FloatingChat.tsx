/**
 * The floating chat (the desktop app's `float_chat`): a round button at the bottom right of every page that opens a
 * sheet with the Moebius agent over the page. The sheet is a borderless card down the right that leaves the page
 * recognisable behind it; Escape, or the control that minimises it, closes it and the focus goes back to the button
 * that opened it. Minimising turns it back into the button and the chat stays across pages. Every message carries
 * where the person is, which only the agent is told; the first one starts a chat of origin `assistant` (the Chat page
 * leaves it out of its list), in the repository of the page when it has one, and the later ones go to the same chat.
 * A chat whose agent went away is taken up again with what was said in it, as on the Chat page; one the agent ended
 * is left behind, and the next message starts a new conversation.
 */
import { ArrowSquareOutIcon, ChatCircleDotsIcon, MinusIcon, PlusIcon } from "@phosphor-icons/react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link, useLocation } from "react-router";
import { snapshot as readSnapshot } from "../api/calls/board";
import { chats as readChats, say, startChat } from "../api/calls/chat";
import { keys } from "../api/keys";
import { Tag } from "../ui/chat/Tag";
import { Button } from "../ui/components/Button";
import { Composer } from "./Composer";
import { isOpen, isWorking, statusTag, takesMessage } from "./format";
import { pageContext } from "./pageContext";
import { Starters } from "./Starters";
import { ThreadView } from "./ThreadView";
import { useThread } from "./useThread";

export const FLOATING_AGENT = "Moebius";

export function FloatingChat() {
  const queryClient = useQueryClient();
  const location = useLocation();
  const snapshot = useQuery({ queryKey: keys.snapshot, queryFn: () => readSnapshot() });
  const chats = useQuery({ queryKey: keys.chats, queryFn: readChats });
  const [shown, setShown] = useState(false);
  const [chatId, setChatId] = useState<number | null>(null);
  const [message, setMessage] = useState("");
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // The thread is read for as long as the chat it belongs to is remembered, not only while the sheet is shown:
  // minimising it and opening it again finds the conversation where it was, not an empty box refilled from the server.
  const thread = useThread(chatId);
  const opener = useRef<HTMLButtonElement>(null);
  const openedOnce = useRef(false);
  // What the composer holds right now, which the closure of a send cannot see: what is typed while an answer is on its
  // way is the next question, not something to clear away with the one that was sent.
  const held = useRef(message);
  const write = (value: string) => {
    held.current = value;
    setMessage(value);
  };

  const chat = chats.data?.find((each) => each.id === chatId);
  // A chat that was just started is not in the list yet, so while its record is on the way the message still goes to
  // it rather than opening a second chat.
  const writesHere = chat ? takesMessage(chat.status) : chatId !== null;
  // Whether the agent is still up: a message to a chat whose agent went away starts that agent again on the same chat.
  const up = chat !== undefined && isOpen(chat.status);
  // Where the person is now: sent with every message, and said under the sheet's own name, because the page behind it
  // is partly covered and is the thing being asked about.
  const where = pageContext(location.pathname, snapshot.data);
  const working = chat !== undefined && isWorking(chat.status);
  // The same words the Chat page uses for the state of the chat, so the sheet does not look alive when its agent is
  // gone: the next message takes the chat up again rather than starting a new one.
  const tag = chat ? statusTag(chat.status, chat.asks) : null;

  // Escape closes the sheet wherever the focus is on the page. Once it closes, the focus goes back to the button that
  // opened it, so a keyboard does not lose its place.
  useEffect(() => {
    if (!shown) return;
    const onKeyDown = (event: KeyboardEvent) => {
      // A control that took the key itself — a menu or a dialog over this sheet — closes first, and the sheet stays.
      if (event.key === "Escape" && !event.defaultPrevented) setShown(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [shown]);

  useEffect(() => {
    if (shown) openedOnce.current = true;
    else if (openedOnce.current) {
      openedOnce.current = false;
      opener.current?.focus();
    }
  }, [shown]);

  async function send() {
    const text = message.trim();
    if (text === "" || sending) return;
    setSending(true);
    setError(null);
    setSent((n) => n + 1);
    try {
      if (chatId !== null && writesHere) {
        await say(chatId, text, where.text);
        // Taking a chat up again says something new in its tag and in the list of chats, so both are read again
        // instead of waiting for the events to say so.
        if (!up) await queryClient.invalidateQueries({ queryKey: keys.chats });
      } else {
        const run = await startChat({
          agent: FLOATING_AGENT,
          message: text,
          context: where.text,
          origin: "assistant",
          ...(where.repository === null ? {} : { repository: where.repository }),
        });
        setChatId(run.id);
        await queryClient.invalidateQueries({ queryKey: keys.chats });
      }
      if (held.current.trim() === text) write("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't send the message. Try again.");
    }
    setSending(false);
  }

  if (!shown) {
    return (
      <Button
        type="button"
        variant="primary"
        ref={opener}
        aria-label="Open the floating chat"
        title="Ask the agent about this page"
        onClick={() => setShown(true)}
        className="fixed right-6 bottom-6 z-sheet size-12 rounded-full p-0"
      >
        <ChatCircleDotsIcon aria-hidden="true" className="size-5" weight="fill" />
      </Button>
    );
  }

  return (
    <section
      role="dialog"
      aria-label="Floating chat"
      className="fixed top-6 right-6 bottom-6 z-sheet flex w-96 max-w-full flex-col gap-3 rounded-card bg-surface p-3 shadow-raised"
    >
      <header className="flex items-center gap-2">
        <div className="min-w-0 flex-1 px-2">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="m-0 min-w-0 break-words text-base font-semibold text-text">
              Moebius agent
            </h2>
            {tag && <Tag tone={tag.tone}>{tag.label}</Tag>}
          </div>
          <p className="m-0 min-w-0 break-words text-xs text-secondary">viewing {where.name}</p>
        </div>
        {/* Both belong to a conversation that exists: nothing is left behind, and nowhere else to open, when this is
            one nobody has spoken in yet. */}
        {chatId !== null && (
          <>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="New conversation"
              title="New conversation"
              onClick={() => {
                setChatId(null);
                setError(null);
              }}
            >
              <PlusIcon aria-hidden="true" className="size-5" weight="bold" />
            </Button>
            <Button asChild variant="ghost" size="icon" title="Open in Chat">
              <Link
                to={`/chat/${chatId}`}
                aria-label="Open in Chat"
                onClick={() => setShown(false)}
              >
                <ArrowSquareOutIcon aria-hidden="true" className="size-5" weight="bold" />
              </Link>
            </Button>
          </>
        )}
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Minimise the floating chat"
          title="Minimise the floating chat"
          onClick={() => setShown(false)}
        >
          <MinusIcon aria-hidden="true" className="size-5" weight="bold" />
        </Button>
      </header>
      <div className="min-h-0 flex-1" data-testid="floating-conversation">
        <ThreadView
          entries={chatId === null ? [] : thread.entries}
          working={working}
          sent={sent}
          empty="Ask the agent about this page, or start from one of these. It is told where you are."
          // The starters are for the moment before anything is written; once a message is on its way they are in the way.
          emptyAction={message.trim() === "" ? <Starters onPick={write} /> : undefined}
          compact
        />
      </div>
      <Composer
        value={message}
        onChange={write}
        onSend={() => void send()}
        canSend={message.trim() !== "" && !sending}
        error={error}
        placeholder="Ask the agent"
        autoFocus
        compact
      />
    </section>
  );
}
