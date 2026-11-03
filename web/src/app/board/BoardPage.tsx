/**
 * The board of a repository: its columns of the default workflow with their cards (dragged only where a person may
 * move them), the filters, the inspector dock, the form of a new issue and a shell in the repository's clone, in a sheet over the
 * board. The
 * column of work in progress is the point of the page, so everything else is quiet: the header counts in one sentence,
 * the filters are a row of labelled fields, a column says what its cards are doing in a word, and an empty column says
 * what will arrive there. Everything comes from the snapshot, which the event stream keeps current
 * (`live/invalidation.ts`), so a card moves on this page when the server moves it. The chosen card and the filters are
 * in the address (`?card=`, `?column=`, `?agent=`, `?status=`), so a reload and a link keep them.
 */
import { PlusIcon, TerminalWindowIcon } from "@phosphor-icons/react";
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useNavigate, useParams, useSearchParams } from "react-router";
import { snapshot } from "../../api/calls/board";
import { keys } from "../../api/keys";
import type { Board, Card, InboxItem } from "../../api/types/board";
import { Button } from "../../ui/components/Button";
import {
  type DropResult,
  KanbanBoard,
  KanbanCard,
  KanbanCardFace,
  KanbanCards,
  KanbanHeader,
  KanbanProvider,
} from "../../ui/components/KanbanBoard";
import { KanbanCardContent } from "../../ui/components/KanbanCardContent";
import { Loader } from "../../ui/components/Loader";
import { Empty } from "../../ui/page/Empty";
import { Notice } from "../../ui/page/Notice";
import { PageHeader } from "../../ui/page/PageHeader";
import { ReadFailed } from "../../ui/page/ReadFailed";
import { ScrollFades } from "../../ui/page/ScrollFades";
import { boardPath } from "../pages";
import { ColumnMark } from "./ColumnMark";
import { Dock } from "./Dock";
import { type Decision, useDecisions } from "./decisions";
import { FilterBar } from "./FilterBar";
import {
  agentsOf,
  boardColumns,
  cardBadges,
  cardDetail,
  cardStatus,
  changeOf,
  dropRule,
  type Filters,
  groupByColumn,
  inboxOf,
  inspectedCard,
  issueUrl,
  nextCard,
  pullUrl,
} from "./model";
import { NewIssueDialog } from "./NewIssueDialog";
import { RepositoryTerminal } from "./RepositoryTerminal";

/**
 * What a column holds when it is empty: one sentence for how a card comes to it. There is no title, because the
 * column's header is already the name of the place, and a heading repeating it is a second label on the same thing.
 */
const COLUMN_EMPTY: Record<string, string> = {
  Backlog: "Issues you create wait here until they are approved.",
  "Not started": "Approved issues wait here until an agent takes one.",
  "In progress": "A card arrives here when an agent starts a run on it.",
  "In review": "A card arrives here when an agent has a change to review.",
  "Ready to merge": "A card arrives here when its change passes its checks.",
  Done: "A card arrives here once its change is merged.",
};

const ANY_COLUMN = "A card arrives when the column's script moves one in.";

export function BoardPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const data = useQuery({ queryKey: keys.snapshot, queryFn: () => snapshot() });
  const { decide, pending, notice, setNotice } = useDecisions();
  const [creating, setCreating] = useState(false);
  const [terminalOpen, setTerminalOpen] = useState(false);
  // The card a send-back needs a reason for, not a count: a count stays raised once it is raised, so every card
  // opened afterwards found the reason form waiting for it.
  const [rejectFor, setRejectFor] = useState<number | null>(null);

  const board = data.data?.boards.find((b) => String(b.repository.id) === id);
  const inbox = data.data?.inbox ?? [];
  const filters = useMemo<Filters>(
    () => ({
      column: params.get("column") ?? "",
      agent: params.get("agent") ?? "",
      status: params.get("status") ?? "",
    }),
    [params],
  );
  // The cards are grouped once per read of the board and once per change of the filter, not once per render of the
  // dock or of a decision coming back.
  const groups = useMemo(
    () => (board ? groupByColumn(board, inbox, filters) : []),
    [board, inbox, filters],
  );

  if (data.isPending) return <Loader message="Reading the board" className="py-8" />;
  // A read that failed is not a board that does not exist, and the page that says so would send the person off to
  // choose a project that is already chosen.
  if (data.isError) {
    return (
      <>
        <PageHeader title="Board" />
        <ReadFailed
          error={data.error}
          fallback="The board could not be read. Try again."
          busy={data.isFetching}
          onRetry={() => void data.refetch()}
        />
      </>
    );
  }
  if (!data.data || !board) {
    return (
      <>
        <PageHeader title="Board" />
        <Empty
          title="There is no such board"
          body="Pick a project in the sidebar to open its board."
        />
      </>
    );
  }
  const setQuery = (changes: Record<string, string>) =>
    setParams(
      (current) => {
        const next = new URLSearchParams(current);
        for (const [key, value] of Object.entries(changes)) {
          if (value === "") next.delete(key);
          else next.set(key, value);
        }
        return next;
      },
      { replace: true },
    );
  const inspected = inspectedCard(board, inbox, params.get("card"));
  function onDragEnd(result: DropResult) {
    // A decision is on its way: the buttons are disabled for the same moment, and a drag is another way to ask for one.
    if (pending) return;
    if (!result.column) return;
    const card = board?.cards.find((c) => String(c.id) === result.cardId);
    if (!card) return;
    const to = result.column;
    const drop = dropRule(card, to, inboxOf(inbox, card));
    const number = card.number;
    let decision: Decision | null = null;
    if (drop.kind === "approve") decision = { kind: "approve", card: card.id, number };
    if (drop.kind === "retry") decision = { kind: "retry", card: card.id, number };
    if (drop.kind === "merge") decision = { kind: "merge", change: drop.change, number };
    if (decision) {
      setQuery({ card: String(card.id) });
      decide(decision);
    } else if (drop.kind === "reject") {
      // Sending a change back needs a reason: the dock asks for it, and only for this card.
      setQuery({ card: String(card.id) });
      setRejectFor(card.id);
    } else if (drop.kind === "refused") {
      setNotice({ tone: "error", text: `Cannot move #${number} to ${to}: ${drop.reason}` });
    }
  }

  /** The card under the pointer while it is dragged. */
  function renderOverlay(cardId: string) {
    const card = board?.cards.find((c) => String(c.id) === cardId);
    if (!board || !card) return null;
    const status = cardStatus(card, inboxOf(inbox, card), board);
    return (
      <KanbanCardFace
        name={`#${card.number} ${card.title}`}
        isLive={status === "live"}
        className="cursor-grabbing"
      >
        <CardContent board={board} inbox={inbox} card={card} cards={board.cards} />
      </KanbanCardFace>
    );
  }

  return (
    <>
      {/* A card open takes a column of the page for its drawer, and the head of the page is part of the page: its
          actions stand left of the drawer rather than under it. */}
      <div className={inspected ? "lg:pr-96" : undefined}>
        <PageHeader
          title={board.repository.name}
          actions={
            <>
              <Button variant="secondary" onClick={() => setTerminalOpen(true)}>
                <TerminalWindowIcon className="size-4" aria-hidden="true" />
                Terminal
              </Button>
              <Button onClick={() => setCreating(true)}>
                <PlusIcon className="size-4" weight="bold" aria-hidden="true" />
                New issue
              </Button>
            </>
          }
        />
      </div>
      {/* The dock stands beside the whole of the board, not beside its lanes: it belongs to the page, and starting it
          under the filters spent the room it asks for and left the filters standing alone over nothing. */}
      <div className="flex min-w-0 flex-col gap-4 lg:flex-row lg:items-start">
        <div className="flex min-w-0 flex-1 flex-col">
          <FilterBar
            repositories={data.data.boards.map((b) => b.repository)}
            repository={board.repository.id}
            onRepository={(repository) => navigate(boardPath(repository))}
            columns={boardColumns(board)}
            agents={agentsOf(board)}
            filters={filters}
            onChange={(next) => setQuery({ ...next })}
          />
          {notice && (
            <Notice
              tone={notice.tone}
              className="mb-4"
              action={
                <Button variant="ghost" size="sm" onClick={() => setNotice(null)}>
                  Dismiss
                </Button>
              }
            >
              {notice.text}
            </Notice>
          )}
          <ScrollFades className="flex-1 pb-3">
            <KanbanProvider onDragEnd={onDragEnd} renderOverlay={renderOverlay}>
              {groups.map(({ column, cards, total }) => (
                <BoardColumn
                  key={column}
                  board={board}
                  inbox={inbox}
                  name={column}
                  cards={cards}
                  total={total}
                  busy={pending}
                  inspectedId={inspected?.id}
                  onSelect={(card) => setQuery({ card: String(card.id) })}
                />
              ))}
            </KanbanProvider>
          </ScrollFades>
        </div>
        {inspected && (
          // The column holds the drawer's width so the board keeps the room it has; the drawer itself runs the whole
          // height of the window and is flush to its top, right and bottom edges — it is the page's own card inspector
          // rather than a sheet laid over it — and scrolls its sections rather than the page scrolling them. It is a
          // drawer as long as two of the board's columns still fit beside it, which `lg` is the width for; under that
          // there is no board left to look at through a fixed panel, so the dock takes the row under the columns.
          <div className="w-full shrink-0 lg:w-96">
            <div className="lg:fixed lg:inset-y-0 lg:right-0 lg:w-96">
              <Dock
                key={inspected.id}
                board={board}
                card={inspected}
                item={inboxOf(inbox, inspected)}
                busy={pending}
                askFor={rejectFor}
                onAskAnswered={() => setRejectFor(null)}
                onDecide={decide}
                onClose={() => {
                  setQuery({ card: "none" });
                  setRejectFor(null);
                }}
              />
            </div>
          </div>
        )}
      </div>
      <RepositoryTerminal
        repository={board.repository.id}
        open={terminalOpen}
        onOpenChange={setTerminalOpen}
      />
      <NewIssueDialog
        repository={board.repository.id}
        name={board.repository.name}
        open={creating}
        onOpenChange={setCreating}
        onCreated={(card) => setNotice({ tone: "ok", text: `Created "${card.title}"` })}
      />
    </>
  );
}

function BoardColumn({
  board,
  inbox,
  name,
  cards,
  total,
  busy,
  inspectedId,
  onSelect,
}: {
  board: Board;
  inbox: InboxItem[];
  name: string;
  cards: Card[];
  total: number;
  /** While a decision is on its way, when no card can be moved. */
  busy: boolean;
  inspectedId: number | undefined;
  onSelect: (card: Card) => void;
}) {
  return (
    <KanbanBoard name={name}>
      <KanbanHeader
        name={name}
        count={cards.length}
        total={total}
        mark={<ColumnMark name={name} />}
      />
      <KanbanCards className="min-h-12">
        {cards.map((card) => {
          const status = cardStatus(card, inboxOf(inbox, card), board);
          return (
            <KanbanCard
              key={card.id}
              id={String(card.id)}
              name={`#${card.number} ${card.title}`}
              testId={`card-${card.number}`}
              column={name}
              dragDisabled={busy}
              isSelected={inspectedId === card.id}
              isLive={status === "live"}
              onOpen={() => onSelect(card)}
            >
              <CardContent
                board={board}
                inbox={inbox}
                card={card}
                cards={cards}
                onOpen={() => onSelect(card)}
              />
            </KanbanCard>
          );
        })}
        {/* The column is already the surface under these words; a card of its own inside it is a second one, and one
            that reads as a card which has lost its words. */}
        {cards.length === 0 && <Empty plain centered body={COLUMN_EMPTY[name] ?? ANY_COLUMN} />}
      </KanbanCards>
    </KanbanBoard>
  );
}

/** What a card shows, on the board and on the drag overlay. */
function CardContent({
  board,
  inbox,
  card,
  cards,
  onOpen,
}: {
  board: Board;
  inbox: InboxItem[];
  card: Card;
  cards: Card[];
  /** Opens the card; left out on the drag overlay, where the card is only a picture of itself. */
  onOpen?: () => void;
}) {
  const status = cardStatus(card, inboxOf(inbox, card), board);
  const change = changeOf(board, card);
  const coding = card.column === "In progress" || card.column === "In review";
  const next = card.column === "Not started" ? nextCard(cards) : undefined;
  return (
    <KanbanCardContent
      displayId={`#${card.number}`}
      issueUrl={issueUrl(board.repository, card)}
      title={card.title}
      preview={card.body}
      detail={cardDetail(card, board.round_limit)}
      badges={cardBadges(card, status, next === card.id)}
      pullRequest={
        change?.pull_request != null
          ? {
              number: change.pull_request,
              url: pullUrl(board.repository, change.pull_request),
              status: change.state,
            }
          : null
      }
      rounds={coding ? { done: card.rounds, limit: board.round_limit } : null}
      isLoading={status === "live"}
      muted={card.column === "Done"}
      onOpen={onOpen}
    />
  );
}
