/**
 * The floating chat (the desktop app's `float_chat`): a round button at the bottom right of every page that opens a
 * small window with the Moebius agent over the page. Minimising turns it back into the button and the chat stays
 * across pages. Every message carries where the person is, which only the agent is told; the first one starts a chat
 * of origin `assistant` (the Chat page leaves it out of its list), in the repository of the page when it has one, and
 * the later ones go to the same chat. A chat that ended is left behind: the next message starts a new one.
 */
import { ArrowSquareOutIcon, ChatCircleDotsIcon, MinusIcon, PlusIcon } from "@phosphor-icons/react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useLocation } from "react-router";
import { snapshot as readSnapshot } from "../api/calls/board";
import { chats as readChats, say, startChat } from "../api/calls/chat";
import { keys } from "../api/keys";
import { IconButton } from "../ui/components/IconButton";
import { Composer } from "./Composer";
import { isOpen, isWorking } from "./format";
import { pageContext } from "./pageContext";
import { type HeldPicture, holdPictures, imageBodies, takesImages } from "./pictures";
import { ThreadView } from "./ThreadView";
import { useChoices } from "./useChoices";
import { useThread } from "./useThread";

export const FLOATING_AGENT = "Moebius";

export function FloatingChat() {
  const queryClient = useQueryClient();
  const location = useLocation();
  const snapshot = useQuery({ queryKey: keys.snapshot, queryFn: () => readSnapshot() });
  const chats = useQuery({ queryKey: keys.chats, queryFn: readChats });
  const choices = useChoices();
  const [shown, setShown] = useState(false);
  const [chatId, setChatId] = useState<number | null>(null);
  const [message, setMessage] = useState("");
  const [pictures, setPictures] = useState<HeldPicture[]>([]);
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const thread = useThread(shown ? chatId : null);

  const chat = chats.data?.find((each) => each.id === chatId);
  const going = chat !== undefined && isOpen(chat.status);
  const working = chat !== undefined && isWorking(chat.status);
  const looks = takesImages(
    choices.available,
    chat?.model,
    choices.agents.find((each) => each.name === FLOATING_AGENT)?.model ?? null,
  );

  async function send() {
    const text = message.trim();
    if (text === "" || sending) return;
    setSending(true);
    setError(null);
    setSent((n) => n + 1);
    const where = pageContext(location.pathname, snapshot.data);
    const held = imageBodies(pictures);
    try {
      if (chatId !== null && going) {
        await say(chatId, text, where.text, held);
      } else {
        const run = await startChat({
          agent: FLOATING_AGENT,
          message: text,
          context: where.text,
          origin: "assistant",
          ...(held.length === 0 ? {} : { images: held }),
          ...(where.repository === null ? {} : { repository: where.repository }),
        });
        setChatId(run.id);
        await queryClient.invalidateQueries({ queryKey: keys.chats });
      }
      setMessage("");
      setPictures([]);
    } catch (e) {
      setError(e instanceof Error ? e.message : "cannot send the message");
    }
    setSending(false);
  }

  /** Holds the pictures the person pasted or dropped, and says what it had to leave out. */
  async function hold(files: File[]) {
    const taken = await holdPictures(files, pictures.length);
    if (taken.held.length > 0) setPictures((held) => [...held, ...taken.held]);
    if (taken.refused.length > 0) setError(taken.refused[0] ?? null);
  }

  if (!shown) {
    return (
      <button
        type="button"
        aria-label="Open the floating chat"
        title="Ask the agent"
        onClick={() => setShown(true)}
        className="fixed right-[18px] bottom-[18px] z-40 flex size-12 items-center justify-center rounded-full bg-accent text-on-fill shadow-lg transition-transform hover:scale-105"
      >
        <ChatCircleDotsIcon className="size-6" weight="fill" />
      </button>
    );
  }

  return (
    <section
      role="dialog"
      aria-label="Floating chat"
      className="fixed right-[18px] bottom-[18px] z-40 flex h-[min(34rem,calc(100vh-36px))] w-[min(26rem,calc(100vw-36px))] flex-col gap-base rounded-xl border border-line-strong bg-surface p-base shadow-lg"
    >
      <header className="flex items-center gap-half">
        <span className="flex-1 truncate px-base text-sm font-bold text-high">Moebius agent</span>
        <IconButton
          icon={PlusIcon}
          aria-label="New conversation"
          title="New conversation"
          onClick={() => {
            setChatId(null);
            setPictures([]);
            setError(null);
          }}
        />
        {chatId !== null && (
          <Link
            to={`/chat/${chatId}`}
            aria-label="Open in Chat"
            title="Open in Chat"
            onClick={() => setShown(false)}
            className="flex items-center justify-center rounded-sm p-half text-low hover:bg-secondary/50 hover:text-normal"
          >
            <ArrowSquareOutIcon className="size-icon-sm" weight="bold" />
          </Link>
        )}
        <IconButton
          icon={MinusIcon}
          aria-label="Minimise the floating chat"
          title="Minimise"
          onClick={() => setShown(false)}
        />
      </header>
      <div className="min-h-0 flex-1" data-testid="floating-conversation">
        <ThreadView
          entries={chatId === null ? [] : thread.entries}
          working={working}
          sent={sent}
          empty="Ask the agent about this page. It is told where you are."
        />
      </div>
      <Composer
        value={message}
        onChange={setMessage}
        onSend={() => void send()}
        canSend={message.trim() !== "" && !sending}
        running={working}
        error={error}
        pictures={looks === false ? [] : pictures}
        onHold={looks === false ? undefined : (files) => void hold(files)}
        onUnhold={
          looks === false
            ? undefined
            : (picture) => setPictures((held) => held.filter((each) => each.key !== picture.key))
        }
        placeholder="Ask the agent"
        autoFocus
      />
    </section>
  );
}
