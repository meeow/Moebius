/**
 * One turn of a conversation (the desktop app's CH3-D): the person's message as its head and what the agent did and
 * answered as its body. Presentational: the turn comes in as data (see `chat/turns.ts`), and so do the folded state
 * and the clicks. A later page that shows a run's conversation draws it with this.
 *
 * The answer is body text on a readable measure. What the agent did is one row apiece — thinking, a tool, a group of
 * reads, a changed file, a todo list — each a control 40px tall that says its state in a word, opens to the text it
 * worked with, and keeps that text on the colour of code rather than in a box of its own colours.
 */
import { CircleNotchIcon } from "@phosphor-icons/react";
import {
  inputPath,
  inputSummary,
  refused,
  type ToolCall,
  toolTitle,
  verdict,
} from "../../chat/conversation";
import type { Block, StepView, Turn as TurnData } from "../../chat/turns";
import { cn } from "../lib/cn";
import { plural } from "../lib/format";
import { ChatAggregatedToolEntries } from "./ChatAggregatedToolEntries";
import { ChatAssistantMessage } from "./ChatAssistantMessage";
import { ChatCollapsedThinking } from "./ChatCollapsedThinking";
import { ChatErrorMessage } from "./ChatErrorMessage";
import { ChatFileEntry } from "./ChatFileEntry";
import { ChatTodoList } from "./ChatTodoList";
import { ChatToolSummary } from "./ChatToolSummary";
import { ChatUserMessage } from "./ChatUserMessage";
import { CopyButton } from "./CopyButton";
import { DiffBody, type DiffChange, diffStats } from "./DiffBody";
import { Markdown } from "./Markdown";
import type { ToolStatusLike } from "./ToolStatusDot";

/** The most lines of a tool's output drawn; the rest is said to be left out. */
const MAX_OUTPUT_LINES = 200;

/** The status a tool call is in, told by one of the words `ToolStatusDot` knows. */
export function toolStatus(call: ToolCall): ToolStatusLike {
  if (refused(call)) return { status: "denied" };
  if (call.execution?.state === "finished") {
    return { status: call.execution.failed ? "failed" : "success" };
  }
  return { status: "created" };
}

/** The changes a file entry shows: an edit's replacements, or a written file as all added. */
function changesOf(call: ToolCall): DiffChange[] | null {
  const input = call.input;
  if (input.type === "edit") return input.changes;
  if (input.type === "write") return [{ oldText: "", newText: input.content }];
  return null;
}

/** What a tool was given and what it printed, on the colour of code, each with a Copy button. */
function Details({ call }: { call: ToolCall }) {
  const check = verdict(call);
  const output = call.output.split("\n");
  const shown = output.slice(0, MAX_OUTPUT_LINES).join("\n");
  const input =
    call.input.type === "command"
      ? call.input.command
      : call.input.type === "other"
        ? call.input.json
        : (inputPath(call.input) ?? inputSummary(call.input));
  return (
    <div
      data-testid="tool-details"
      className="reveal-copy mt-1 flex flex-col gap-3 rounded-control bg-code p-3"
    >
      {check && !check.allowed && (
        <p className="break-words text-sm font-semibold text-error-text">
          Blocked: {check.reason || "a rule refused this call"}
        </p>
      )}
      {input && (
        <div className="flex items-start gap-3">
          <pre className="min-w-0 flex-1 break-all whitespace-pre-wrap font-mono text-sm text-text">
            {input}
          </pre>
          <CopyButton text={input} label="Copy input" />
        </div>
      )}
      {call.output !== "" && (
        <div className="flex flex-col gap-2 border-t border-hairline pt-3">
          <div className="flex items-start gap-3">
            <pre className="max-h-64 min-w-0 flex-1 overflow-auto break-all whitespace-pre-wrap font-mono text-sm text-text tabular-nums">
              {shown}
            </pre>
            <CopyButton text={call.output} label="Copy output" />
          </div>
          {output.length > MAX_OUTPUT_LINES && (
            <p className="m-0 text-sm text-secondary tabular-nums">
              {plural(output.length - MAX_OUTPUT_LINES, "more line", "more lines")} left out
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function ToolLine({
  call,
  expanded,
  onToggle,
}: {
  call: ToolCall;
  expanded: boolean;
  onToggle: () => void;
}) {
  const status = toolStatus(call);
  const check = verdict(call);
  const changes = changesOf(call);
  if (call.input.type === "todos") {
    return <ChatTodoList todos={call.input.todos} expanded={expanded} onToggle={onToggle} />;
  }
  if (changes && (call.input.type === "edit" || call.input.type === "write") && !refused(call)) {
    const { additions, deletions } = diffStats(changes);
    return (
      <ChatFileEntry
        filename={call.input.path}
        additions={additions}
        deletions={deletions}
        status={status}
        expanded={expanded}
        onToggle={onToggle}
      >
        <DiffBody changes={changes} />
      </ChatFileEntry>
    );
  }
  const hasDetails =
    call.output !== "" || (check !== null && !check.allowed) || call.input.type !== "unknown";
  return (
    <div>
      <ChatToolSummary
        summary={inputSummary(call.input)}
        detail={inputPath(call.input) ?? undefined}
        toolName={toolTitle(call)}
        status={status}
        isTruncated={hasDetails}
        expanded={expanded}
        onToggle={onToggle}
      />
      {expanded && <Details call={call} />}
    </div>
  );
}

function StepLine({
  step,
  isOpen,
  onToggle,
}: {
  step: StepView;
  isOpen: (key: string) => boolean;
  onToggle: (key: string) => void;
}) {
  if (step.kind === "thinking") {
    return (
      <ChatCollapsedThinking
        entries={[{ content: step.text, expansionKey: step.key }]}
        seconds={step.seconds}
        ongoing={step.ongoing}
        expanded={isOpen(step.key)}
        onToggle={() => onToggle(step.key)}
        renderMarkdown={({ content, className }) => (
          <Markdown content={content} className={className} keepNewlines />
        )}
      />
    );
  }
  if (step.kind === "reads") {
    return (
      <ChatAggregatedToolEntries
        label="Read"
        unit="file"
        entries={step.calls.map((call, index) => ({
          summary: inputSummary(call.input),
          detail: inputPath(call.input) ?? undefined,
          status: toolStatus(call),
          expansionKey: `${step.key}-${index}`,
        }))}
        expanded={isOpen(step.key)}
        onToggle={() => onToggle(step.key)}
      />
    );
  }
  return (
    <ToolLine call={step.call} expanded={isOpen(step.key)} onToggle={() => onToggle(step.key)} />
  );
}

function BlockView({
  block,
  isOpen,
  onToggle,
}: {
  block: Block;
  isOpen: (key: string) => boolean;
  onToggle: (key: string) => void;
}) {
  switch (block.kind) {
    case "text":
      return (
        <ChatAssistantMessage
          content={block.text}
          renderMarkdown={({ content }) => (
            // The answer keeps the measure of the standard and takes every line the agent wrote; the wrapper the
            // component copied is one box that says the same.
            <div className="max-w-prose">
              <Markdown content={content} className="break-words" keepNewlines />
            </div>
          )}
          actions={<CopyButton text={block.text} label="Copy message" />}
        />
      );
    case "error":
      return (
        <ChatErrorMessage
          title={block.source === "model" ? "The model returned an error" : "The run failed"}
          content={block.message}
          actions={<CopyButton text={block.message} label="Copy error" />}
        />
      );
    case "mcpError":
      return (
        <ChatErrorMessage
          title={`MCP server ${block.server} could not be reached`}
          content={block.message}
          actions={<CopyButton text={block.message} label="Copy error" />}
        />
      );
    case "steps":
      // A row of the log is 32px rather than the 40px of a thing you aim at, and the rows sit 4px apart, so a log of
      // what the agent did reads as one block of small marks rather than a list to choose from. It is the same rhythm
      // in the page and in the dock. The marker is what tells `e2e/design/audit.test.ts` these rows are a log at 32px
      // rather than controls at 40px.
      return (
        <div className="flex flex-col gap-1" data-testid="steps" data-audit="log">
          {block.steps.map((step) => (
            <StepLine key={step.key} step={step} isOpen={isOpen} onToggle={onToggle} />
          ))}
          {block.working && (
            <p
              className="flex items-center gap-3 px-3 text-sm font-semibold text-progress-text"
              data-testid="working"
            >
              <CircleNotchIcon aria-hidden="true" className="size-4 animate-spin" />
              Working
            </p>
          )}
        </div>
      );
  }
}

interface TurnProps {
  turn: TurnData;
  /** The keys of the steps that are unfolded. */
  unfolded: ReadonlySet<string>;
  onToggle: (key: string) => void;
  /** A tighter rhythm between the blocks and the rows, for the floating chat's dock. */
  compact?: boolean;
}

export function Turn({ turn, unfolded, onToggle, compact = false }: TurnProps) {
  const isOpen = (key: string) => unfolded.has(key);
  const head = turn.head;
  // A turn is written on the canvas rather than stacked in boxes: the person's message is a band of the sunken
  // colour, and what the agent did, answered and got wrong is the text and the rows under it, in the order it came. A
  // card of its own would put a second shadow inside the chat, which is itself a card on the page or a sheet over it.
  const view = (block: Block) => (
    <BlockView key={block.key} block={block} isOpen={isOpen} onToggle={onToggle} />
  );
  return (
    <div data-testid="turn" className={cn("flex min-w-0 flex-col", compact ? "gap-3" : "gap-4")}>
      {head && (
        <ChatUserMessage
          content={head.message}
          title={head.said === "steered" ? "You steered the chat" : "You"}
          className="rounded-card"
          headerRight={<CopyButton text={head.message} label="Copy your message" />}
          renderMarkdown={({ content }) => (
            <>
              {head.pictures.length > 0 && (
                <ul className="mb-2 flex flex-wrap gap-2" data-testid="person-pictures">
                  {head.pictures.map((picture) => (
                    <li key={picture.key}>
                      <img
                        src={picture.src}
                        alt={picture.name ?? "A picture you sent with the message"}
                        title={picture.name ?? undefined}
                        loading="lazy"
                        className="max-h-24 max-w-40 rounded-control border border-hairline bg-field object-contain"
                      />
                    </li>
                  ))}
                </ul>
              )}
              <p
                className="max-w-prose break-words whitespace-pre-wrap text-base leading-relaxed text-text"
                data-testid="person-message"
              >
                {content}
              </p>
            </>
          )}
        />
      )}
      {turn.blocks.map(view)}
    </div>
  );
}
