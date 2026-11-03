# Set up agents

## When you want this

You want the default workflow to have a Coder, and perhaps a Reviewer, or you want another agent for a purpose of your
own: a stronger model for hard cards, a reviewer that is stricter than the coder, an agent to chat with. An agent is a
named setup that a run uses: the model, how it thinks, its instructions, its skills and extensions, and a token budget.

Agents are kept in one of two places. In tables of `moebius.toml`, which the steps below use. Or in a harness
configuration repository, a git repository of their own, where an edit is a commit and agents can be changed through the
API (see [In a harness configuration repository](#in-a-harness-configuration-repository)). `moebius init` makes one
next to `moebius.toml` and names it in `[harness]`, and a server that keeps its agents in `moebius.toml` can move them
into one from the app (see [Setting it up from the app](#setting-it-up-from-the-app)). Skills and MCP servers are
edited, and MCP servers given to agents, in the repository only. The keys of `moebius.toml` are
listed exactly in [`docs/reference/configuration.md`](../reference/configuration.md), those of the repository in
[`docs/reference/harness-configuration.md`](../reference/harness-configuration.md).

## The Moebius agent

The Moebius agent is bundled and always there. It does for the person in a chat what they could do in the desktop app,
and builds what they ask for on Moebius. It knows Moebius from this guide and the references, which are its skill, and it
learns the state of a server from the server itself:

- It operates Moebius with the [command line](command-line.md): it reads the boards, the inbox, chats, usage, agents,
  rules and settings, and it changes them (an agent, a rule, a skill, an MCP server, a setting, a repository, an issue, a
  brief, an item of the inbox). Its runs get the server's address and their own run token, which the desktop API accepts
  for the Moebius agent, so every decision it makes is recorded as `agent:Moebius` with the chat's run. It cannot store
  an API key or open a shell in a worktree or on the server.
- It builds files: in a repository, what belongs to that repository (its `.moebius/`); in the harness configuration
  repository (see [Build an extension or skill in a chat](#build-an-extension-or-skill-in-a-chat)), agents, skills,
  extensions, MCP servers and the server's rules. It tries a column script with `moebius check-script` and a plugin with
  `cargo test`. The person offers the files as a change between its answers ("Offer changes" in the desktop app,
  `moebius chat offer <id>`), and they are offered when the chat ends; a person merges the change (see
  [Chat with an agent](#chat-with-an-agent)). It never commits, pushes or merges its own files.

Its skills are the reference library and one skill per kind of task, each with the steps, the exact commands and how to
try the result. Pi shows the agent each skill's description and the agent reads a skill when the task matches:

| Skill | For |
|---|---|
| `moebius` | The guide and the references: every command, route, `card` field, script function and configuration key. |
| `moebius-operate` | Reading the state and acting on it: board, cards, inbox, chats, usage, repositories, issues, research, grillings. |
| `moebius-configure` | Agents, rules, a column's script through the API, settings, skills and MCP servers. |
| `moebius-workflows` | Column scripts and whole workflows in a repository's `.moebius/`. |
| `moebius-extensions` | Pi extensions. |
| `moebius-plugins` | Decider plugins in Rust. |
| `moebius-report` | Sending a fix or a problem in Moebius itself to its official repository, after the person says yes. |

## Where Moebius runs, and Moebius's own repository

At the start of every run the Moebius agent (and only it) is given a section "Where Moebius runs" at the end of its
instructions. It says where the server is: its program and version, the `moebius.toml` it runs with, its data
directory, the source checkout it was built from when it lies in one, the addresses of the desktop API and the router,
and its computer and system. The server found these when it started. It also says where the desktop app is: its
program, version, computer and system, the address it reaches the server by and the SSH host if it connects over SSH.
The app reports this to the server (`PUT /v1/client`) when it connects and again whenever the connection is made again,
so a new address shows in the next run; before the app has reported, the section says so. No token or key is in it.
`"$MOEBIUS" api GET /v1/server` and `"$MOEBIUS" api GET /v1/client` read the same facts while a chat goes on.

The section also names https://github.com/Code000-211/Moebius as Moebius's own repository. When the person fixes
something in Moebius itself, or the agent meets a problem in Moebius rather than in the person's project, it offers to
send it there: a pull request for a fix, an issue for a problem. It asks first, shows what it would send and sends
nothing without the person's yes (the `moebius-report` skill has the steps; it needs the `gh` command, else it hands the
person the text to paste).

They are built in and read only: the Skills page and `GET /v1/skills` list them as built in, and the Agents page shows
them, with the built-in instructions, on the Moebius agent (`GET /v1/agents/Moebius` answers both).

Its built-in instructions say this, name the task skills, and say that it does only what the person asked for: it does something hard to undo
(deleting, merging, restarting the server) only when asked for exactly that, never weakens a rule, never writes a secret
anywhere, and ends each answer with what it did, how it tried it, and what the person has to do now.

It is the default of a chat: `POST /v1/chats` without `agent` starts a chat with it. It needs a model. Without
configuration it takes the Coder's model and protocol, and without a Coder the model chosen for the chat; with an
`[agents.Moebius]` table it uses that table's:

```toml
[agents.Moebius]
model = "my-hosted-model"
protocol = "anthropic"
thinking = "high"
instructions = "Write scripts in the style of the repository's other scripts."
```

The keys are those of any agent below. Its `instructions` are added after the built-in ones, and its `skills` after the
built-in skills, which it always has. With neither `[agents.Moebius]` nor `[agents.Coder]`, a chat with it is refused
with status 422 and a message that says which to configure. The skills are written to `<data_dir>/skills/<name>/` at
every start, so a change to the guide arrives with the server.

### Asking it from any page

In the desktop app the round button at the bottom right of every page opens a chat with the Moebius agent over the
page. The agent is told where you are with each message (the page, the repository, the card or chat that is open, and
the script, rules or agent being edited with its text), so "why does this column wait?" on a column's script means that
script. A chat started on a board works in that repository; elsewhere it works in none. The chat stays while you move
between pages; "Open in Chat" shows it on the Chat page, and "New conversation" starts over. A client does the same with
the optional `context` of `POST /v1/chats` and `POST /v1/chats/{id}/messages` (see the
[desktop API](../reference/desktop-api.md)): the agent is told it before the message, and the chat shows only the
message.

## The Grilling partner

The Grilling partner is bundled too: a chat with `agent` set to `Grilling-partner` in a repository questions an idea until the
work is clear and ends in issues you confirm, or in a research report (see [Grilling](grilling.md)). Like the Moebius
agent it takes the Coder's model unless `[agents.Grilling-partner]` gives it one, and its `instructions` are added after the
built-in ones.

## The engineering skills

The Coder, the Reviewer and the Grilling partner are given skills from [Matt Pocock's skills](https://github.com/mattpocock/skills)
(MIT license), which Moebius brings itself: the Coder `tdd` (red, then green, one slice at a time, tests at the public
seams) and `diagnosing-bugs` (build a tight feedback loop before guessing), the Reviewer `code-review` (the Standards
and the Spec of a change, reported apart), the Grilling partner `grilling` and `to-tickets`. Those skills are written for
a person at the keyboard; a run never waits for one. So every one of these agents also has `moebius-autonomous`, which
says what to do where a skill asks the user: decide, and put the decision and the reason in the run's summary; the
card's issue is the spec; there is no tracker to publish to and no setup skill to run (the Grilling partner writes
`.moebius/grilling/issues.json` as before).

- They apply to agents with these names, on both harnesses, in addition to the agent's own `skills`. Nothing is needed to
  get them.
- To go without them, set `engineering_skills = false` on the agent (in `agents/<name>.toml` or `[agents.<name>]`; the
  Agents page has the setting for these three agents, `"$MOEBIUS" agent save` the flag `--no-engineering-skills`).
- The Skills page lists them as built in, with the repository and the commit they were copied from. To change one, or to
  take a newer version, save a skill of the same name in the harness configuration repository: it replaces the built-in
  copy for every agent that has it. "Update from upstream" on the skill's page starts a chat with the Moebius agent that
  imports it from upstream into the repository that way.

The exact rules are in [harness configuration](../reference/harness-configuration.md#engineering-skills).

## The steps

1. Add an `[agents.<name>]` table to `moebius.toml`. The name is made of letters, digits, `-` and `_`. Only `model` is
   required.
2. Make sure the router has an upstream for the agent's `protocol` (`openai` by default, or `anthropic`). The agent's
   model name is sent to that upstream as it is written.
3. Restart the server. The file is read once, at start; a change to it does nothing until then. A mistake (an unknown
   key, a bad name) stops the server with a message that names the key.
4. Use the agent: the default workflow starts `Coder` and, when it exists, `Reviewer`; a script can start any agent
   with `moebius.start_run`; a chat can start with any agent.

| Key | What it does |
|---|---|
| `model` | The model's name, given to Pi-Bolt as `--model`. It may name a model class of `[router.classes]` instead, such as `strong`: the router then asks the class's models in order and falls back to the next when an upstream fails (see [the router](../reference/router.md#model-classes)). |
| `protocol` | `openai` or `anthropic`: which router endpoint the harness uses. |
| `thinking` | Given to Pi-Bolt as `--thinking`, such as `low`, `medium` or `high`. Pi-Bolt checks the value. |
| `instructions` | Text given to Pi-Bolt as `--append-system-prompt`. |
| `skills` | Paths, each given to Pi-Bolt as `--skill`. |
| `extensions` | Paths, each given to Pi-Bolt as `--extension` before the bridge. See [Extensions](extensions.md). |
| `token_budget` | The most tokens (input plus output) the card's runs may use, the teacher's included. Over it the router answers HTTP 429 and the default scripts mark the card stuck. |
| `harness` | `pi-bolt` (the default) or `claude-code`: which harness runs the agent. See below. |
| `runner` | The name of a `[[runners]]` machine the agent's runs go to over SSH; without it they run on the server. See [Running on another machine](running-moebius.md#the-steps) (step 9). |

A relative path in `skills` or `extensions` is relative to the directory that holds `moebius.toml`. Moebius does not
check that the paths exist.

### An agent on Claude Code

An agent can run on Claude Code's command line instead of Pi-Bolt. Install `claude` on the server (or name it with
`[claude_code] program`), and give the agent `harness = "claude-code"`. Claude Code speaks only Anthropic's protocol, so
the agent needs `protocol = "anthropic"` and an `anthropic` upstream in `[[router.upstreams]]`; it has no extensions.

```toml
[agents.Reviewer]
model = "claude-sonnet-4-5"
protocol = "anthropic"
harness = "claude-code"
instructions = "Review the change against the issue."
```

The run is `claude -p` in the card's worktree, with the router as its provider and the run's token as its key, the
agent's model and instructions, its skills in a configuration directory of its own and its MCP servers. Claude Code has
no bridge, so the router checks each tool call in the model's answers against the rules before Claude Code sees it, and
puts a text naming the rule in place of a refused call; the agent usually ends its turn with that text. The run's events,
summary, steering and stopping are the same as for Pi-Bolt (stopping kills the process). A chat with such an agent works,
except its terminal interface, which only Pi-Bolt has, and the reasoning level, which Claude Code is not given. The exact
command line is in [configuration](../reference/configuration.md#claude-code), the check in
[the router](../reference/router.md#the-check-of-a-harness-without-a-bridge).

## In a harness configuration repository

The server can keep its agents and rules in a git repository instead of `moebius.toml`. That is one place for them with a
history, and the desktop API can change them: every edit is a commit. With `[harness]` set, `[agents.*]` and `rules_file`
in `moebius.toml` are errors, so the repository is the only source.

1. Make a git repository with the layout below and push it somewhere the server can reach. A GitHub repository needs the
   GitHub App to have write access to it, if edits through the API are wanted.
2. Name it in `moebius.toml`, as shown below.
3. Start the server. It clones the repository to `<data_dir>/harness` and reads the agents from its default branch. Every
   30 seconds it fetches and reads them again when something changed, so a commit pushed by hand arrives without a
   restart.

```toml
[harness]
url = "https://github.com/acme/harness-configuration"
```

An agent is two files, `agents/<name>.toml` for the settings and `agents/<name>.md` for the instructions, which is
optional. The name is made of letters, digits, `-` and `_`.

<!-- agent-file -->
```toml
model = "my-local-model"
thinking = "medium"
token_budget = 2000000
skills = ["house-style"]
extensions = ["extensions/log-tool-calls.ts"]
```

An agent can be given MCP servers, which are described in `mcp.toml` of the repository and named in the agent's `mcp`
key. A server is a command (`command`, `args`, `env`) or a remote `url` (`headers`), and takes secrets from the server's
environment with `${NAME}` for the names listed in `mcp_env` of `[harness]`:

<!-- skip -->
```toml
[docs]
command = "docs-server"
env = { API_KEY = "${DOCS_TOKEN}" }
```

With `mcp_env = ["DOCS_TOKEN"]` in `moebius.toml` and `mcp = ["docs"]` in the agent's file, its runs reach the server
through the MCP gateway, Moebius's one MCP server for every harness, which starts `docs-server` itself and gives the run
its tools as `docs__<tool>` (in Pi, `mcp__moebius__docs__<tool>`). Every call is checked by the rules like any tool
call, and recorded as an `mcp_call` run event with what the server answered. A run is given exactly its agent's skills
and servers, and never the server's values; see the [reference](../reference/harness-configuration.md#mcptoml).

`agents/Coder.md`:

```markdown
Keep changes small. Run the repository's tests before you stop.
```

A Reviewer on a hosted model needs only its settings, since the instructions are optional:

<!-- agent-file -->
```toml
model = "my-hosted-model"
protocol = "anthropic"
thinking = "high"
```

`skills` names directories under `skills/` that hold a `SKILL.md` (here `skills/house-style/SKILL.md`), `extensions` names
files in the repository, and `mcp` names servers in `mcp.toml`. Unknown keys, and names that are not in the repository, are
errors. The repository's `rules.md` holds the server's rules, in the format of [Rules](rules.md).

An agent whose files are broken is left out and the others go on: `GET /v1/agents` lists it with an `error` and an empty
`model`, the event `harness_error` reports it, and a run that asks for it fails with the error. Fix the file and commit;
the next refresh reads it again.

Change an agent through the API instead of by hand: the edit is checked, committed as `Moebius` and pushed to the
repository. If the push is refused, nothing changes and the request fails with git's message.

```sh
curl -s -H "Authorization: Bearer $TOKEN" $API/v1/agents/Coder
curl -s -X PUT -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"settings": {"model": "my-local-model", "thinking": "high"}, "instructions": "Keep changes small."}' $API/v1/agents/Coder
curl -s -X DELETE -H "Authorization: Bearer $TOKEN" $API/v1/agents/Strong
curl -s -H "Authorization: Bearer $TOKEN" $API/v1/skills
curl -s -H "Authorization: Bearer $TOKEN" $API/v1/mcp-servers
```

The answer of `PUT` carries `commit`, the short id of the commit. `GET /v1/skills` and `GET /v1/mcp-servers` list what an
agent can name, for an editor (the desktop app's Agents screen uses them); they show no secret and are empty without
`[harness]`. The desktop app edits agents this way.

The agent is used from the next run on. The exact files, keys and errors are in the
[reference](../reference/harness-configuration.md).

Without `[harness]` the three routes of an agent answer 422: the agents are in `moebius.toml`.

### Setting it up from the app

On a server without `[harness]`, the Skills, MCP servers and Agents pages of the desktop app, and the step "Keep agents,
skills and MCP servers in a repository" of Get started, have a "Set up" button. Only a person can press it (the
Moebius agent is refused). It sends `POST /v1/harness/setup`, and the server:

1. makes `harness/` next to `moebius.toml` a git repository on `main`, refused when that directory has files;
2. writes each `[agents.<name>]` as `agents/<name>.toml` and `agents/<name>.md`, copies each skill directory an agent
   names to `skills/<directory name>/` and each extension to `extensions/<file name>`, copies the rules file to
   `rules.md`, adds an empty `mcp.toml` and a `README.md`, checks that every agent reads back, and commits;
3. replaces `[agents.*]` and `rules_file` in `moebius.toml` with `[harness] url` naming the repository, keeping the rest
   of the file and its comments.

The app then restarts the server, which clones the repository and reads the agents from it. A skill whose directory has
the name of a built-in skill, two skills or extensions with the same name, or a path that is not there stops the setup
with a message, and nothing is changed. The files the old paths named stay where they were; the agents use the copies
from now on. The same setup with curl:

```sh
curl -X POST -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7171/v1/harness/setup
curl -X POST -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7171/v1/server/restart
```

## Examples

### A Coder and a Reviewer on a local model

```toml
[[router.upstreams]]
protocol = "openai"
base_url = "http://127.0.0.1:8080"

[agents.Coder]
model = "my-local-model"
thinking = "medium"
instructions = "Keep changes small. Run the repository's tests before you stop."
token_budget = 2000000

[agents.Reviewer]
model = "my-local-model"
thinking = "high"
instructions = "Be strict about missing tests."
token_budget = 500000
```

### An agent on a hosted model

The key is read from an environment variable, never from the file. If the variable is not set when the server starts,
the server does not start. A key typed into the desktop app is kept by name in `secrets.toml` of the data directory, and
the file names it with `api_key_secret = "<name>"` instead of `api_key_env`.

```toml
[[router.upstreams]]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[agents.Strong]
model = "my-hosted-model"
protocol = "anthropic"
thinking = "high"
```

### Skills and extensions

A skill is what Pi-Bolt documents as one: a directory with a `SKILL.md` that tells the agent how to do a kind of work,
which it loads when the task calls for it. An extension is TypeScript that runs inside the Pi process.

```toml
[agents.Coder]
model = "my-local-model"
skills = ["skills/house-style"]
extensions = ["extensions/log-tool-calls.ts"]
```

### Use a stronger agent for large cards

The default `code.luau` always starts `Coder`. This version starts `Strong` for a card whose title contains `[large]`
when that agent exists, and `Coder` otherwise. The token budget is the chosen agent's.

<!-- script: code.luau; test: agent-by-title -->
```luau
local function agent_for(card)
  if string.find(card.title, "[large]", 1, true) ~= nil and moebius.has_agent("Strong") then
    return "Strong"
  end
  return "Coder"
end

function on_card(card)
  local run = card.run
  local agent = agent_for(card)
  local budget = moebius.agent_budget(agent)
  if run ~= nil and (run.status == "starting" or run.status == "running") then
    moebius.wait("coding")
  elseif run ~= nil and run.purpose == "code" and run.status == "succeeded" and card.note == nil then
    moebius.move("In review")
  elseif card.stopped then
    moebius.stuck("stopped by a person")
  elseif card.rounds >= moebius.round_limit then
    moebius.stuck("the round limit is reached")
  elseif budget ~= nil and card.tokens >= budget then
    moebius.stuck("the token budget of " .. budget .. " is spent")
  elseif moebius.day_budget ~= nil and moebius.tokens_today() >= moebius.day_budget then
    moebius.wait("the daily token budget is spent")
  else
    moebius.start_run(agent, "code", card.note)
    if card.rounds == 0 then
      moebius.wait("coding")
    else
      moebius.wait("coding, round " .. (card.rounds + 1))
    end
  end
end
```

Commit it as `.moebius/code.luau` (see [Column scripts](column-scripts.md)). A card is stuck when its tokens reach the
budget of the agent chosen at that moment.

## What the default workflow expects of the agents

### The Coder

The agent named `Coder` must exist, or every card stops in `In progress` with a `board_error` saying
`no agent named Coder`. A round is a `code` run in a worktree of its own. Its prompt is the issue (its number, title
and body), then: make the change, run what checks the repository has, stop when the issue is done, and Moebius commits what is
left in the worktree. After a send-back it ends with `Notes from the last round:` and the reason.

Moebius does the commit. A `code` run whose worktree has nothing uncommitted when the agent stops has failed with
`the agent changed nothing`, even if the agent made a commit of its own. So do not tell the Coder to commit.

### The Reviewer

The agent named `Reviewer` is optional. With it, every change is reviewed before it is offered, along two axes that are
two runs of a round; without it changes are offered unreviewed. The **Standards** run checks the change against the
repository's own rules (`CONTRIBUTING.md`, `CLAUDE.md`, `AGENTS.md` and an optional rubric `.moebius/review.md`) and a
fixed baseline of code smells, which are always judgement calls ("Possible Feature Envy"). The **Spec** run checks it
against the issue. Both are told to read the change on its branch and not to edit files, to report each finding as
`FINDING blocking: <text>` or `FINDING judgement: <text>` and an `EVIDENCE:` line quoting the hunk or the line of the
issue, and to end with one last line, `VERDICT: approve` or `VERDICT: changes`.

Both axes use the `Reviewer` unless the configuration has an agent `Standards` or `Spec`: give that agent another
model (or a model class) to review on a model other than the Coder's. Only blocking findings send the card back; a
finding without evidence is dropped. Judgement calls are attached to the offered change.

The rest describes a run with purpose `review`, which a script of your own can still start. Its final answer, the
text of the last assistant message, is read like this:

- The last line decides. `VERDICT: approve` (the word in any case) approves; the lines before it are the findings.
- `VERDICT: changes` sends the card back to the Coder, and the findings are its note. A `changes` without findings
  becomes `The review asked for changes but gave no findings.`
- Anything else, including no verdict line or text after it, is `changes` with the finding `The review gave no verdict.`
  Work that was not reviewed is not approved.

If you give the Reviewer `instructions`, keep them from adding closing remarks after the verdict line.

### Any other agent

A column script starts it by name with `moebius.start_run(agent, purpose, note)`. The purpose is any string. Only
`code`, `review` and `review-standards` are counted by the default scripts, and a purpose other than the three review
purposes is prompted like `code`. A
fourth argument names the `[[runners]]` machine the run goes to in place of the agent's own `runner`:
`moebius.start_run(agent, purpose, note, "gpu-box")`.

## Chat with an agent

A chat is a run without a card: you write, the agent answers, and you can go on, steer and stop. Any agent in
`moebius.toml` can be chatted with, and the Moebius agent, which a chat without `agent` is with.

```sh
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"agent": "Coder", "message": "Explain how the router chooses an upstream.", "repository": 1}' $API/v1/chats
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"message": "Write a ready.luau that starts a card only when nothing blocks it.", "repository": 1}' $API/v1/chats
```

With `repository` the agent works in a new worktree on the branch `moebius/chat-<run id>`. The chats list says how many
files the worktree changes (`changed_files`). To offer them, without ending the chat, use `POST /v1/chats/{id}/offer` (the
desktop app's "Offer changes (n files)", or `moebius chat offer <id>`) once the agent has answered: what the agent left is
committed there and, as the branch then has commits that the default branch does not, offered as a change without a
card: an inbox item of kind `ready_to_merge`, the event `change_offered` and, on GitHub, a pull request titled as the
chat is. The chat goes on in the same worktree; offer again after more work and the same change gets the newer commit. A
person merges it with `POST /v1/changes/{id}/merge`. Ending the chat offers what it left in the same way. A chat that
changed nothing offers nothing (`422`), and while the agent works the offer is refused (`409`). Without `repository`, the
agent works in an empty directory and nothing is offered. The answer is the run; its `id` is the chat's id. Read what the
agent says as run events and send the next message:

```sh
curl -s -H "Authorization: Bearer $TOKEN" "$API/v1/runs/$CHAT/events?after=0"
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"message": "Now the budgets."}' $API/v1/chats/$CHAT/messages
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"reason": "the budgets are in"}' $API/v1/chats/$CHAT/offer
```

A chat can run on another model and reasoning level than its agent's: `model` and `thinking` (`off`, `minimal`, `low`,
`medium`, `high`, `xhigh` or `max`) in `POST /v1/chats`, and `POST /v1/chats/{id}/settings` to change them while the
chat waits (`409` while the agent works). Pi is started with them every time, so they last after the terminal or a long
wait; a new reasoning level is sent to the waiting Pi with `set_thinking_level`, and a new model starts the chat's Pi
again on the same session, as Pi knows only the model it was started with. Without `message` a chat starts and waits,
so its terminal can be opened at once. In the desktop app these are the pickers under the field to write in, and the
"Terminal" button next to Send.

```sh
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"agent": "Coder", "model": "qwen3-coder", "thinking": "high"}' $API/v1/chats
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"thinking": "low"}' $API/v1/chats/$CHAT/settings
```

A message can carry pictures as well as words, for a model that looks at them. `images` in both `POST /v1/chats` and
`POST /v1/chats/{id}/messages` is a list of `{"name", "media_type", "data"}`, the bytes in base64, `name` what the
person's clipboard or file was called; at most 6 to one message, each one of the four kinds a chat keeps and none too
large for it. Which
models take them is the `images` of `GET /v1/models/available`; a message with a picture to a chat whose model answers
in words alone is refused (`422`), as is a picture of another kind or of too many bytes. The chat keeps the bytes
itself, outside any worktree, so a pasted picture never lands in what the chat offers. `GET /v1/chats/{id}/images/{file}`
gives the bytes back to show, and the `said` event names the file instead of repeating them. In the web app the
paperclip, a paste into the field and a file dragged onto it all attach a picture, and the picture control is hidden for
a model that cannot look at one.

A chat is not ended in the desktop app: it waits for the next message as long as you like. One that has waited 30
minutes, with no terminal open, has its Pi ended to free it, and stays `waiting`; the next message starts Pi again on the
chat's own session, so the agent knows the whole conversation. The command line and the API still end a chat
(`moebius chat end <id>`, `POST /v1/chats/{id}/end`), which offers its work a last time and removes its worktree, and
"Stop" (`POST /v1/runs/{id}/stop`) ends one whose agent is working.

### A chat in Pi-Bolt's own terminal interface

A person can also work with a waiting chat in Pi-Bolt's own terminal interface: in the desktop app, the "Terminal"
button beside "Send" (and "Chat" under the terminal to go back); over the API, the WebSocket `GET /v1/chats/{id}/tui`. It is Pi-Bolt's
interactive mode in the chat's worktree with the chat's own model, provider, instructions, skills, MCP servers,
extensions and bridge, so every tool call is still checked by the rules and recorded as a decision of the chat. It is
the same conversation as the chat: the terminal continues the chat's Pi session, so the agent there knows what was said
in the chat; what is said in the terminal is recorded as the chat's events as it happens (the person's messages as
`said`, the agent's text, thinking and tool calls as Pi records them), so the chat view, the Fleet and the event stream
show it; and when the terminal closes, the chat's own process continues the same session, so the next message in the
chat knows what was said in the terminal. The chat takes no message while the terminal is open (`409`), and it opens
only between turns: `409` while the agent works. The files the terminal changes are in the chat's worktree and are
committed and offered with the chat's offer, or when it ends, as the agent's are. See the [desktop API](desktop-api.md#get-v1chatsidtui).

To build on the harness configuration (a skill, an extension, an MCP server, an agent, the server's rules) in a chat,
see [Build an extension or skill in a chat](#build-an-extension-or-skill-in-a-chat).

A chat does not count against `run_limit`; the daily budget applies to it and its `token_budget` counts the chat's own
tokens. Every tool call in a chat is checked by the built-in rules and the server's rules. See the
[desktop API](desktop-api.md) for the rest.

## Import skills and MCP servers

Skills and MCP servers that someone else wrote can be imported instead of typed in. On the Skills page or the MCP
servers page of the desktop app, press **Import**:

1. Choose **From a zip** and pick the file (it is sent to the server with `POST /v1/uploads`, at most 20 MB), or **From
   a link** and paste a git repository or a page that leads to one.
2. Add a note if it helps: which of the skills or servers you want, which key it needs.
3. Press **Ask the agent**. A chat with the Moebius agent opens in the floating window. The agent unzips or clones the
   source in its working directory, writes each skill with `PUT /v1/skills/{name}` or each server with `PUT
   /v1/mcp-servers/{name}` (every one a commit), and ends with what it imported and what is left to you.

The agent never replaces an existing skill or server without asking, and never imports a skill under a built-in name.
It writes a skill's other text files, such as its scripts, with `PUT /v1/skills/{name}/files/{path}`; a file that is
not text (an image, a font) is named in its answer, to be added in a chat in the repository (below). On the Skills page
a skill is a list of its files: pick `SKILL.md` or another file to edit it, or **New file** to add one; each save or
delete is a commit. An MCP server takes keys as `${NAME}`: set `NAME` in the server's environment and keep it in
`harness.mcp_env`. Import needs a harness configuration repository; without one the button is disabled. The same
works without the app: upload the zip with `curl --data-binary @skills.zip -H 'content-type: application/zip'
$API/v1/uploads` and start a chat that names the `path` it answers.

## Build an extension or skill in a chat

When the server has a harness configuration repository (`[harness]`), a chat can work in it instead of in a repository.
The agent then writes the files of the configuration, and the person merges them like any change.

1. Start the chat with `harness` instead of `repository`. The Moebius agent is the default; say what the agent should
   build and which agent should have it:

```sh
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"message": "Add a skill testing-habits to skills/testing-habits/SKILL.md and give it to the Coder in agents/Coder.toml.", "harness": true}' $API/v1/chats
```

2. The agent works in a worktree of the clone, on the branch `moebius/chat-<run id>`. A skill is
   `skills/<name>/SKILL.md`; an extension is a TypeScript file, `extensions/<name>.ts`; an agent names them in
   `skills` and `extensions` of `agents/<name>.toml` (see [harness configuration](../reference/harness-configuration.md)).
3. Offer the work (`POST /v1/chats/{id}/offer`, "Offer changes" in the desktop app), or end the chat
   (`POST /v1/chats/{id}/end`). Moebius checks that the branch changes only configuration files and that
   every agent can still be read. If so, the work is offered as a change of the harness configuration repository, with a
   `ready_to_merge` item in the inbox. If not, the offer is refused with why (`422`), or, when the chat ends, the run gets
   a `moebius_error` event saying why, and the branch stays in the clone.
4. Merge it with `POST /v1/changes/{id}/merge`. The merge is pushed to `origin`, the agents are read again, and the next
   run of the Coder has the skill. Try it by starting a chat with that agent.

## Keep tool outputs small

What tools print is most of what a run sends the model, and most of it is read once. Every agent on Pi-Bolt has a table
`tool_output` that keeps it small, with defaults that need no configuration (Pi-Bolt's bridge does it; an agent on
Claude Code has no bridge, and the table has no effect there):

- The output of a tool call older than the last `hide_after_turns` turns (default 10) is replaced, in the requests to the
  model, by `[bash {"command":"cargo test"}: output hidden: 312 lines]`. Errors are kept. The run's events and Pi's
  session keep every output.
- One result is at most `max_lines` lines (200) and `max_bytes` bytes (16384). Above that the model reads its head and
  tail around `[1,840 lines left out; the full output is in .moebius/out/3.txt]`; the agent can read that file in
  ranges. `.moebius/out/` is never committed.
- With `compress` (on), escape codes, progress bars and repeated lines go, and the output of `cargo test`, `pytest`,
  `go test`, `jest` and `vitest` is cut to the failing tests and the summary, a cargo build's to its errors and warnings.
  Nothing that is not known to be noise is removed.

```toml
[agents.Coder]
model = "my-local-model"

[agents.Coder.tool_output]
hide_after_turns = 20     # 0 keeps every output
max_lines = 400           # 0 is no limit
max_bytes = 32768
compress = true
```

In a harness configuration repository the same table goes in `agents/<name>.toml`, and the agent editor of the desktop
app has a field for each (shown for an agent on Pi-Bolt). Each result made smaller is a `tool_output` run event with its bytes before and after; the
Usage page shows what they saved beside the longest tool waits. A built-in rule, `no-dumping-large-files`, refuses `cat`
of a file over 64 KB and tells the agent to read a range or grep instead (see [Rules](rules.md)). The exact behaviour is
in the [configuration reference](../reference/configuration.md) and the [bridge protocol](../reference/bridge-protocol.md).

## How to try an agent

Start a chat with it and read its events. A first message that makes it call a tool shows the whole path: model request
through the router, tool call through the bridge (a `tool_check` event), tool result. The agent's answer is in the data of the event
of kind `agent_end`, which is Pi's own record.

## What can go wrong and how you see it

| What happens | How you see it |
|---|---|
| The server does not start. | The message names the key: an unknown key, a bad agent name, an upstream whose `api_key_env` variable is not set or whose `api_key_secret` is not stored, `[agents.*]` together with `[harness]`, or a harness repository that cannot be cloned. |
| An agent of the harness repository is missing from the chats. | It is listed by `GET /v1/agents` with an `error`, and the event `harness_error` carried it. |
| `no agent named X` | In a column script, a `board_error` event; in `POST /v1/chats`, status 422. The name is case sensitive. |
| Model requests fail. | The router answers 502 when it has no upstream for the agent's `protocol`, or cannot reach it. See the [router reference](../reference/router.md). |
| Model requests are refused with 429. | The card's `token_budget` or the day's budget is spent. The message says which. |
| A run fails at once. | The run is `failed` and its events hold a `moebius_error` with the reason, for example that Pi-Bolt could not be started (`pi_bolt.program`). A run's standard error is in `<data_dir>/runs/<id>/stderr.log`. |
| The model says it cannot see an earlier output. | It was hidden after `tool_output.hide_after_turns` turns; the agent can run the command again, or you raise or turn off the window (`0`). |
| A skill or extension does not load. | Moebius only passes the path. Look in `<data_dir>/runs/<id>/stderr.log` and in Pi-Bolt's documentation. |
| The Reviewer never approves. | Its final answer has text after the verdict line, or no verdict line. Read the run's `summary` (`GET /v1/runs/{id}`). |
