# Desktop API

The desktop API is how the desktop app, or any client such as `curl`, reads the boards and acts on them. This page is for
anyone who writes a client or a script against a running server. The code is in `crates/moebius-core/src/server/api/`,
anyone who writes a client or a script against a running server. The code is in `crates/moebius-core/src/server/api/`,
one module per part; the events are published from `app/`, `harness/pibolt/`, `decide/check/`, `repos/poll/`,
`improve/retro/`, `improve/trial/`, `server/loops.rs` and `server/schedule.rs`.

The API is HTTP with JSON bodies plus one WebSocket, served on `listen` (default `127.0.0.1:7171`). Keep it on a private
network.

## Authentication

Every request to `/v1/`, the WebSocket upgrade included, carries `Authorization: Bearer <token>` (or, in a browser, the
cookie of a [session](#post-v1session)). The token is the content of
`token_file` (by default `<data_dir>/token`, created when the server first starts). A missing or wrong token is answered
with `401` and the plain text `a valid token is needed`. The exceptions are `POST /v1/webhooks/{forge}`, which a forge
sends and which carries the forge's signature instead, and `/v1/session`, which signs a browser in. The paths outside
`/v1/` need no token: they are the [web app](#the-web-app).

The run token of a run of the Moebius agent (`MOEBIUS_TOKEN` in its environment, see the [bridge
protocol](bridge-protocol.md#the-environment-of-the-harness)) is accepted as well, while the run lasts: it is how the
agent operates Moebius with the [command line](cli.md). A request made with it acts as the agent: every decision it
records has the decider `agent:Moebius` instead of `person`, and the run is the judgement's `run` (for a decision about
another run, such as a steer, the judgement keeps that run, and the agent's run is in the context as `acting_run`).
The run token of any other agent, and a revoked one, is answered with `401`. Two routes stay with a person and answer
`403` with `only a person can do this` to a run token: `PUT /v1/secrets/{name}` (an agent must not set keys),
`GET /v1/webhooks` and `POST /v1/webhooks/{forge}/secret` (they hold the webhook secrets),
`POST /v1/session` and `POST /v1/session/codes` (a run token never signs in or makes a login code), `POST /v1/harness/setup` (moving the agents is a person's choice) and `GET /v1/runs/{id}/terminal`, `GET /v1/chats/{id}/tui`, `GET /v1/terminal` and `GET /v1/repositories/{id}/terminal` (the shells) and `GET /v1/ports/{port}/tunnel`, and `PUT /v1/client` (what the app says of itself). Where a route below says "a person" and "decider
`person`", an agent that uses its token is the decider instead.

A browser signs in once and then carries the cookie `moebius_session` (`HttpOnly; SameSite=Strict; Path=/`, and `Secure` when
a proxy in front of the server says with `X-Forwarded-Proto: https` that the browser used TLS). Every route, the
WebSocket upgrades included, accepts a valid cookie as the person's token, as a person: a person's route is open to it
and the decider of a decision is `person`. A bearer token that is not valid is refused even when the request also has a
valid cookie. Because a browser sends a cookie by itself, a request that only a cookie vouches for has to prove that
it comes from the web app: a method other than `GET`, `HEAD` and `OPTIONS` needs the header `X-Moebius: 1`, and an
`Origin` header, which browsers send on changes and on WebSocket upgrades, must name the host of the `Host` header
(scheme left out). Otherwise the answer is `403` with `cross-site request refused`. A request with the bearer token
needs neither.

```sh
TOKEN=$(cat .moebius/token)
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7171/v1/health
```

## Errors

A failed request has a status code and a plain-text message as its body.

| Status | Meaning |
|---|---|
| `401` | No valid token. |
| `403` | A run token of the Moebius agent on a route that only a person may use. |
| `404` | The repository, card, change or run in the path does not exist. |
| `409` | A merge conflicts: the default branch is left as it was. |
| `422` | The request is understood but not allowed: invalid values, or a card or change in the wrong state. Also what axum answers for a JSON body with a missing or mistyped field. |
| `400`, `415` | The body is not valid JSON, or the content type is not `application/json` (`application/zip` for `POST /v1/uploads`). |
| `413` | The body of `POST /v1/uploads` is larger than 20 MiB. |
| `500` | Anything else: git, the store or a forge failed. |

A request with a body must have `Content-Type: application/json` and a JSON object, `{}` at least. The one exception is
`POST /v1/uploads`, whose body is a zip file.

## Types

These objects appear in the responses. All ids are numbers; times are Unix seconds.

`Repository`: `id`, `name`, `path` (the clone on the server), `forge` (`none`, `github`, `gitlab` or `forgejo`),
`default_branch`, `github` (`owner/repo` or null), `trusted` (array of logins on the forge), `project` (the path on
GitLab, such as `group/project`, or the repository on Forgejo as `owner/repo`, or null).

`Card`: `id`, `repository_id`, `number`, `title`, `body`, `author`, `column`, `created_at`, `note` (why the card was sent
back to coding, or null), `retried_after` (the id of the card's latest run when a person last retried it, or null) and
`confirmed` (true for a card made from a confirmed grilling, which needs no approval).
`Run`: `id`, `card_id` (null for a chat), `agent`, `harness`, `purpose` (`code`, `review`, other purposes of column scripts,
`chat`, `retro`, the Retro analyst's run, `trial`, the trial of a proposal on a card, or `research`, a round of the Autoresearcher), `status` (`starting`, `running`, `waiting`, `succeeded`, `failed` or `stopped`; `waiting` is a chat waiting for
the person's next message), `worktree`, `started_at`, `ended_at`, `summary`, `repository_id` (the repository a chat
works in, or null) and `harness_repository` (true for a chat that works in the [harness configuration
repository](harness-configuration.md#changing-it-in-a-chat); `harness` is the name of the harness that runs the agent)
and `origin` (where a chat was started: `chat`, or `assistant` for the desktop app's floating window; `chat` for every
other run) and `runner` (the name of the [runner](configuration.md#runners) the run's harness goes on, or null for a run on the server) and
`title` (the name a person gave this chat, or null while it is untitled and its `Chat` title comes from its first
message; see `POST /v1/chats/{id}/title`. Only a chat is ever named, so it is null for every other run).

`Grilling`: `id`, `run_id` (the chat), `repository_id`, `issues` (array of `{"title", "body", "blocked_by"}`, where
`blocked_by` is an array of indexes into `issues`; empty for a report and for an empty grilling), `report` (text or null),
`status` (`proposed`, `reported`, `empty`, `confirmed` or `dismissed`), `reason` (why a grilling is `empty`; else empty) and
`at`.

`Chat`: `id` (the id of its run), `agent`, `status` (that of the run), `repository_id` (or null), `harness_repository`
(true for a chat in the harness configuration repository), `started_at`, `ended_at`, `title` (the name a person gave
the chat, see `POST /v1/chats/{id}/title`, or the first line of the first message, at most 80 characters; empty while a
chat started without a message has none), `answered` (the `seq` of the chat's last `agent_settled` or `moebius_error`
event, 0 without one), `seen` (how far a person has read it, see `POST /v1/chats/{id}/seen`) and `changed_files` (how
many files the chat's worktree changes against the default branch of its repository, committed or not, new files
included: what `POST /v1/chats/{id}/offer` would offer; 0 for an ended chat and one without a repository), `asks`
(true while the chat waits and the agent's last answer ends in a question to the person, so it needs an answer) and
`origin`
(`chat`, `assistant` for a chat of the desktop app's floating window, see `POST /v1/chats`, or `automation` for a chat an
automation started, see `POST /v1/automations`), `model` and `thinking`
(the model and reasoning level the chat was given in place of its agent's, see `POST /v1/chats` and
`POST /v1/chats/{id}/settings`; null while it uses its agent's). A chat is unread while `answered` is greater than `seen`.

`Automation`: `id`, `name`, `schedule` (a cron expression of five fields in the server's local time), `agent`, `prompt`,
`repository_id` (the repository its chats work in, or null), `harness_repository` (true when they work in the harness
configuration repository), `enabled`, `created_at`, `next_firing` (when the schedule comes due next, as Unix seconds;
null while it is off or when the schedule never comes due) and `last_firing` (its latest `Firing`, or null).

`Firing`: `id`, `automation_id`, `status` (`started`, `skipped` or `failed`), `run_id` (the chat it started, or null),
`chat_status` (that chat's status now, or null), `manual` (true when a person fired it with
`POST /v1/automations/{id}/run`), `reason` (why it was skipped or failed; else empty) and `at`.

`Change`: `id`, `repository_id` (null for a change of the harness configuration repository), `harness_repository` (true
for such a change), `card_id` (or null), `branch`, `state` (`open`, `merged` or `closed`), `pull_request`
(the number of the pull request on GitHub or Forgejo or of the merge request on GitLab, or null), `checks` (`pending`,
`success`, `failure` or null; on GitLab the latest pipeline of the head commit: `failed` and `canceled` are `failure`,
`success` and `skipped` are `success`, anything else is `pending`; on Forgejo the latest status of each context of the
head commit: any `failure` or `error` is `failure`, else any `pending` is `pending`, else `success`), `review` (the
latest review's state in lower case, such as `approved`; on GitLab `approved` once anyone approved; on Forgejo the
latest review given that was not dismissed, with `REQUEST_CHANGES` as `changes_requested`; or null), `head` (the published commit), `published_at`, `checks_head` (the commit
`checks` are for), `failed_checks` (the failing checks of a pull request, the failed jobs of a pipeline, or the failed
statuses of a commit, one per line, or null) and `check_output` (the
end of the output of the repository's own checks, or null). A change's `branch` is `moebius/card-<number>`, `moebius/chat-<run id>` for the work of a chat, which has no card, or `research/<research id>-<name>` for a lever of a research, which has none either.

`InboxItem`: `id`, `kind`, `card_id` (or null), `title`, `data` (an object that depends on the kind), `resolved_at` (null
in the snapshot, which holds only open items), `at`. Kinds the core adds: `approve_issue` (`data.reason`), `stuck`
(`data.reason`), `package_update` (`data.name`, `data.installed`, `data.version`; no card), `ready_to_merge` (`data.change`, `data.branch`, and `data.judgements`, the review's judgement calls, when it had any), `change_closed` (`data.change`, `data.pull_request`) and
`proposal` (`data.proposal`, `data.branch`, `data.trials`: an array of `{"card", "outcome", "reason"}` for the cards it
addresses, and `data.regression`: the same for the cards of the regression set, empty when none was tried; its `title` is the
proposal's and it has no card; a proposal for Moebius's own configuration has `data.kind` (`settings` or
`repository`), `data.repository` (the repository's id, or null), `data.reason` (the Retro's reason) and `data.diff`
instead of `data.trials`), `grilling_issues` (`data.grilling`, `data.repository`, `data.issues`: an array of `{"title",
"body", "blocked_by"}`; its `title` is "N issues from a grilling" and it has no card) and `grilling_report` (`data.grilling`,
`data.repository`, `data.report`: the text; no card) and `takeover` (`data.takeover`, `data.kind`, `data.agreement`,
`data.count`; its `title` is "The decision model can take over <kind>" and it has no card) and `extension_update`
(`data.package`, `data.from`, `data.to`, `data.source`; its `title` is "<package> <from> -> <to>" and it has no card). The
nightly schedule, from `retro_at` on, once a day, puts one `extension_update` item in the inbox for each installed package
with a newer version, and no second one for a version it has told of; the item is resolved when the package is current or
an update is proposed.
A column script can add other kinds with `moebius.ask`.

## Endpoints

### `GET /v1/health`

Response: `{"version": "0.1.0"}`, the server's version.

### The web app

The server serves the web app, built from `web/` into `web/dist` and embedded in the binary when it is built, at `/` on
the address of this API. Every `GET` of a path outside `/v1/` that is no file of the app answers `index.html`, since the
app routes in the browser; the files get their media type, `index.html` has `Cache-Control: no-cache` and the files
under `assets/` are cached for a year. A binary built without `web/dist` answers a short page that says to run `pnpm -C
web build` and build the server again. Nothing under `/v1/` is a file: an unknown `/v1/` path is answered `401` without
a valid token and `404` with one.

At start, `moebius serve` prints the line `Open the web app: http://<listen address>/?login=<code>` on standard output
(not in the log). The page signs in with the code and takes it out of the address.

### `POST /v1/session`

Signs a browser in. No `Authorization` is needed. Request body: `{"token": string}` with the person's token, or
`{"code": string}` with a one-time code of `POST /v1/session/codes`. Response: `204` and the cookie `moebius_session=<id>`
(see Authentication). The sessions live in memory, so a restart of the server signs everybody out. Errors: `401` with
`a valid token is needed` for a wrong token, a wrong, used or expired code, or a body with neither; `403` with `only a
person can do this` for a run token.

### `GET /v1/session`

`200` with `{"signed_in": true}` for a request with a valid cookie or bearer token, else `401`.

### `DELETE /v1/session`

Signs out: forgets the session of the cookie and clears the cookie. Response: `204`.

### `POST /v1/session/codes`

A new one-time code that signs a browser in without the token: it works once and for 10 minutes. Request body: `{}`.
Response: `{"url": string, "expires_at": number}`, where `url` is `http://<Host of the request>/?login=<code>` (`https` when
the request says with `X-Forwarded-Proto` that it came over TLS) and `expires_at` is Unix seconds. The code is only in
memory, never stored or logged. [`moebius web`](cli.md) prints the address. Errors: `403` to a run token.

### `GET /v1/server`

Where the server runs, found when it started: `{"program": string, "version": string, "config_file": string,
"data_dir": string, "source_dir": string, "api_address": string, "router_address": string, "host": string, "os": string,
"arch": string}`. `program` is the running executable, `config_file` the `moebius.toml` it was started with,
`source_dir` the source checkout the program lies in (`target/` of a cargo workspace; absent otherwise), `api_address`
the base URL of this API and `router_address` the address of the router. No token or key is in it. The Moebius agent is
given the same facts at the start of each of its runs (section "Where Moebius runs" of its instructions). Errors: `404`
a core that was not started as a server.

### `GET /v1/client`

What the desktop app last reported about itself: the body of `PUT /v1/client` and `reported_at`, the time the server
heard it (Unix seconds). Errors: `404` no app has reported since the server started.

### `PUT /v1/client`

The app tells the server where it runs, when it connects and whenever the event stream connects again. Request body:
`{"program": string, "version": string, "os": string, "arch": string, "host": string, "server_address": string,
"ssh_host": string or null}`. `program` is the app's executable, `host` the computer it runs on, `server_address` the
address it reaches this server by (over SSH the local end of the tunnel) and `ssh_host` the host name it reaches it
through over SSH, absent when it connects directly. The server keeps the latest report only, in memory, cuts every
field to 500 characters and redacts it. Response: what `GET /v1/client` answers. The Moebius agent is told it at the
start of each run. Errors: `403` to a run token.

### `GET /v1/snapshot`

Everything the app shows. Response:

```json
{
  "boards": [
    {"repository": {"id": 1, "name": "demo", "path": "/srv/.moebius/repos/demo", "forge": "none",
                    "default_branch": "main", "github": null, "trusted": []},
     "cards": [{"id": 1, "repository_id": 1, "number": 1, "title": "Add a feature", "body": "", "author": "person",
                "column": "Backlog", "created_at": 1790000000, "note": null, "retried_after": null,
                "rounds": 0, "tokens": 0, "run": null, "blocked_by": [], "refusals": 0, "change": null}],
     "round_limit": 5,
     "waiting": {},
     "changes": []}
  ],
  "harness_changes": [],
  "inbox": []
}
```

`harness_changes` are the `Change`s of the harness configuration repository, which belongs to no board.
`boards` has one entry per repository, with all its cards and changes. `round_limit` is the most `code` rounds a card
gets. Each entry of `cards` is a `Card` with what a board shows of it, so that a client needs no other read:

| Field | Meaning |
|---|---|
| `rounds`, `tokens` | The card's `code` runs (and rounds of attempts of which none passed) and the tokens of all its runs since it was last retried, as `card.rounds` and `card.tokens` in [column scripts](column-scripts.md). |
| `run` | The card's latest run that is not an attempt, `{"id", "agent", "purpose", "status"}`, or null when it has none: an attempt that was not picked is shown by `attempts`, not here. A board counts the runs working on it from the cards whose `run.status` is `starting` or `running`. |
| `blocked_by` | The numbers of the issues that block the card and are not done yet, as `card.blocked_by`. |
| `refusals` | How many tool calls of that latest run were refused (its `tool_check` and `mcp_call` run events with the decision `refuse`). |
| `attempts` | The latest group of attempts of a card that was tried several times at once (see [column scripts](column-scripts.md#attempts)), oldest first, each `{"run", "model", "status", "result", "output", "picked", "reason"}`: `model` the model or class the attempt was started with, or null; `status` its run's status; `result` `passed`, `failed` or null before it was judged; `output` why it failed; `picked` whether it was the one kept, and `reason` why. Absent when the card has no attempts. |
| `change` | The card's open change, `{"id", "state", "checks", "review_verdict"}`, or null. `checks` is `pending`, `success`, `failure` or `none`, as `card.change.checks`; `review_verdict` is `approve` or `changes` once a review ended since the latest `code` run, else null. Whether the change conflicts is not in the snapshot. |

A card in `Done` has `rounds` and `tokens` 0, no `run`, no `change`, no blockers and no refusals: the snapshot is read on
every event, and a finished card shows none of it. `waiting` maps the id of each card whose column
script is waiting (`moebius.wait`, `ask`, `stuck`) to the reason, as text; a card that does not wait has no entry. The
reasons are kept in memory, so after a restart they are missing until the board engine's next tick; `card_waiting`
(below) keeps them current. `inbox` holds the items that are not
resolved yet, oldest first.

### `GET /v1/events`

A WebSocket. After the upgrade, every event the core publishes arrives as one JSON text message, see Events below. Events
are not replayed: a client sees only what happens after it connected, so read the snapshot after connecting. Messages the
client sends are ignored; closing the socket ends the stream.

### `POST /v1/repositories`

Adds a repository: the server clones `url` into its data directory and reads the clone's checked-out branch as the
default branch.

Request:

| Field | Type | Meaning |
|---|---|---|
| `name` | string | Required. Letters, digits, `-`, `_` and `.`, and not starting with a dot. Must be unique. |
| `url` | string | Required. Anything `git clone` takes: a URL or a path on the server. Without a forge, an absolute path that does not exist, or an empty folder, is first made a new repository: branch `main` with one empty commit, "Start the repository". |
| `forge` | string | `none` (the default), `github`, `gitlab` or `forgejo`. |
| `github` | string | `owner/repo` on GitHub. Required with the forge `github`, not allowed otherwise. Both parts are made of letters, digits, `-`, `_` and `.`. |
| `project` | string | The project's path on GitLab, groups first: `group/project` or `group/subgroup/project`. Required with the forge `gitlab`. At least two parts, each made of letters, digits, `-`, `_` and `.` and not starting with a dot. With the forge `forgejo`, the repository as `owner/repo`: exactly two such parts. Not allowed otherwise. |
| `trusted` | array of strings | Logins on the forge (GitHub logins, GitLab usernames, Forgejo logins) whose issues need no approval. Only with a forge. |

Response: the `Repository`. Errors: `422` for an invalid name, an absolute path to a folder with files that is not a git
repository (nothing is created then), a `github` that is not `owner/repo`, a `project` that is not a path as above, or
a combination other than "no forge, no `github`, no `project`, no `trusted`", "forge `github` with `github`" , "forge
`gitlab` with `project`" and "forge `forgejo` with `project`"; `422` also when the name is taken (nothing is cloned or created then); `500` when the clone
fails.

```sh
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name": "demo", "url": "/srv/git/demo"}' http://127.0.0.1:7171/v1/repositories
```

For GitHub, with the App set up, the clone authenticates as the App's installation, so a private repository works:

```json
{"name": "widgets", "url": "https://github.com/acme/widgets.git", "forge": "github", "github": "acme/widgets", "trusted": ["leon"]}
```

For GitLab, with `[gitlab]` set up, the clone sends the access token when `url` is on that GitLab, so a private project
works:

```json
{"name": "tools", "url": "https://gitlab.com/acme/platform/tools.git", "forge": "gitlab", "project": "acme/platform/tools", "trusted": ["leon"]}
```

For Forgejo, such as Codeberg, with `[forgejo]` set up, the clone sends the access token when `url` is on that forge:

```json
{"name": "gadgets", "url": "https://codeberg.org/acme/gadgets.git", "forge": "forgejo", "project": "acme/gadgets", "trusted": ["leon"]}
```

### `PUT /v1/repositories/{id}`

A person, or the Moebius agent, replaces the logins whose issues on a repository on a forge need no approval.
Request: `{"trusted": [string], "reason": string}`; `reason` is optional and is kept with the decision. Each login is
trimmed and kept once (in any case); a login must be made of letters, digits and `-_.[]`. The decision is recorded
(kind `edit_repository`, decider `person` or `agent:Moebius`; its context has `repository`, `trusted` and
`previous_trusted`, the logins joined with `, `) and the event `repository_updated` is sent. A list that changes nothing
records nothing. The new list counts for cards checked from then on; a card already approved or waiting stays as it is.
Response: the `Repository`. Errors: `404` no such repository; `422` the repository is on no forge, or a login is blank
or has other characters.

### `DELETE /v1/repositories/{id}`

A person, or the Moebius agent, removes a repository from Moebius: the repository with every card of its board and what
belongs to them (their runs and their transcripts, usage, judgements, replay shadows, inbox and dependencies), its
changes, its chats and grillings, the open inbox items that name its changes (`ready_to_merge`), grillings
(`grilling_issues`, `grilling_report`) or proposals (`proposal`) without belonging to a card, its lessons, its research, its automations and its localize index; and its clone and
worktrees under the server's data directory. The decision is recorded (kind `delete_repository`, decider `person` or
`agent:Moebius`; its context has `repository`) and the event `repository_removed` is sent.

Request: `{"reason": string}`; `reason` is optional and is kept with the decision. A body is not required.

Response: the `Repository`, as it last was. Errors: `404` no such repository; `409` one of its runs is still going —
stop that run first, nothing is removed then.

```sh
curl -s -X DELETE -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7171/v1/repositories/2
```

### `POST /v1/repositories/{id}/issues`

Creates an issue from a person in the app. Its card starts in `Backlog`, with the author `person`.

Without a forge the card is trusted and moves on. On a repository with a forge the issue is opened there first, the same
way a confirmed grilling opens its issues, and the card gets the issue's number right away, so that the poll finds it
existing. The card is `confirmed`, since a person wrote it, and so moves on from `Backlog` without approval, whatever the
repository's `trusted` list says.

Request: `{"title": "...", "body": "..."}`. `title` is required and must not be empty or blank; `body` is optional.

Response: the `Card`. Errors: `404` no such repository; `422` the title is empty; `500` the forge refuses to open the
issue (no card is made).

### `GET /v1/repositories/{id}/scripts/{column}`

The script a column runs for a repository: the repository's own `.moebius/<file>.luau` on its default branch (see
[column scripts](column-scripts.md#repository-overrides)), or the built-in one. `{column}` is the column's name as the
board shows it, in any case and URL-encoded (`In%20progress`); `Ready`, the old name of `Not started`, finds that column too. Response: `{"column": string, "path": string, "source":
"repository" or "built-in", "text": string}`. `path` is where the repository keeps its own script for the column
(`.moebius/ready.luau` for `Not started`; the files are `backlog`, `ready`, `code` for `In progress`, `review`, `merge` and
`done`), whichever `source` is. Errors: `404` no such repository or column.

### `PUT /v1/repositories/{id}/scripts/{column}`

A person, or the Moebius agent, replaces the column's script for the repository. Request: `{"text": string, "reason":
string}`; `reason` is optional and is kept with the decision. The script is checked as `moebius check-script` checks it,
in the column, against a made-up card, and refused when the check fails. Then `.moebius/<file>.luau` is committed
(`Change the <Column> column's script`) on the default branch of Moebius's clone; for a repository on GitHub the clone
is first brought up to date with `origin` and the branch is pushed with the installation's token. The decision is
recorded (kind `edit_script`, decider `person` or `agent:Moebius`; its context has `repository`, `column`, `path`, `text`
and `change`) and the event `script_changed` is sent. Text that changes nothing makes no commit and no decision. Response:
the script as `GET` answers it, now with `source` `repository`. Errors: `404` no such repository or column; `409` the
clone cannot be brought up to date or the push is refused: the commit is undone and the message is git's, with keys
redacted; `422` the check failed, and the message is the check's (it starts with the script's file name).

### `DELETE /v1/repositories/{id}/scripts/{column}`

Removes the repository's own script for the column, committed and pushed the same way (`Use the built-in script for the
<Column> column`, kind `edit_script`), so the column runs the built-in script again. Response: the script as `GET`
answers it, now the built-in one. Errors: `404` no such repository or column, or the repository has no script of its own
for the column; `409` as for `PUT`.

### `GET /v1/repositories/{id}/tree`

A directory of the repository as it is on its default branch (the same branch column scripts are read from: `origin`'s
for a repository on GitHub, the clone's own otherwise). `path` is the directory from the repository's root, with `/`
between directories, given as `?path=src/lib`; left out or empty, it is the root. Response: `{"path": string, "entries": [{"name": string, "path":
string, "kind": "dir" or "file", "size": number}]}`: `path` is the directory without leading or trailing `/`, an
entry's `path` is from the root, `size` is in bytes (0 for a directory); directories come first, each group sorted by
name. Submodules are left out. Errors: `404` no such repository or directory (a file is not one); `422` the path is
absolute, climbs out with `..`, or names `.git` (in any case, or as `.git.`, `.git `, `GIT~1`).

### `GET /v1/repositories/{id}/files`

The paths of every file on the repository's default branch, for the app's "Go to file". Optional `?query=`: only the
paths that contain the characters of the query in that order, ignoring case and surrounding blanks (the app ranks them
itself). Response: `{"paths": [string]}`, from the repository's root, sorted; submodules are left out, and a clone
without a commit has none. Errors: `404` no such repository.

### `GET /v1/repositories/{id}/search`

A full-text search of the files on the default branch, run with `git grep` in Moebius's clone, ignoring case, skipping
binary files. `?q=` is the text to find (required, not blank); `?regex=true` takes it as an extended regular
expression instead of the text itself; `?limit=` is the most hits answered (default 200, at most 1000). Response:
`{"files": [{"path": string, "hits": [{"line": number, "text": string}]}], "total": number, "truncated": bool}`: files
in path order, hits in line order (`line` from 1, `text` cut after 300 characters), `total` the hits in `files`, and
`truncated` true when there are more than the limit. Errors: `404` no such repository; `422` `q` is blank or the
regular expression is invalid (the message is git's).

### `GET /v1/repositories/{id}/files/{*path}`

A file of the repository as it is on its default branch; `{*path}` is the rest of the URL, from the repository's root
(`/v1/repositories/1/files/src/main.rs`). Response, always one shape: `{"path": string, "size": number, "text": string}`
for a UTF-8 file of at most 1 MiB (1048576 bytes); a larger file has `"too_large": true` and a file that is not UTF-8
text, or holds a NUL byte, has `"binary": true`, both without `text`. `binary` and `too_large` are left out when they
are false. Errors: `404` no such repository or file (a directory is not one); `422` as for the tree.

### `PUT /v1/repositories/{id}/files/{*path}`

A person, or the Moebius agent, writes the file, creating it and its directories when they are new. Request: `{"text":
string, "message": string, "reason": string}`; `message` is the commit's message, `Update <path>` when it is left out or
blank, and `reason`, optional, is kept with the decision. The file is committed on the default branch of Moebius's
clone, the same way `PUT /v1/repositories/{id}/scripts/{column}` commits a script: for a repository on GitHub the clone
is first brought up to date with `origin` and the branch is pushed with the installation's token. The decision is
recorded (kind `edit_file`, decider `person` or `agent:Moebius`; its context has `repository`, `path` and `change`, the
commit's message) and the event `file_changed` is sent. Text that changes nothing makes no commit and no decision.
Response: the file as `GET` answers it. Errors: `404` no such repository; `409` as for a script: the clone cannot be
brought up to date or the push is refused, the commit is undone and the message is git's, with keys redacted; `422` the
path is empty, absolute, climbs out with `..`, names `.git` (in any case, or as `.git.`, `.git `, `GIT~1`), is a directory, or goes through a symbolic link in the
clone.

The repository's own rules, `.moebius/rules.md`, are checked first as `PUT /v1/rules` checks the server's: a `## `
line needs a name and no name may be given twice. Text that fails is refused with `422` and a message that starts with
`.moebius/rules.md: ` and says which line; nothing is committed. Saved, the decision has the kind
`edit_repository_rules` instead of `edit_file`, with the text in its context.

### `PUT /v1/cards/{id}`

A person, or the Moebius agent, changes a card's title and body, in any column. Request: `{"title": string, "body":
string, "reason": string}`; `body` and `reason` are optional (`body` left out is empty), and `reason` is kept with the
decision. The title is trimmed and must not be blank. On a repository with a forge the issue is changed there first
(for GitHub, `PATCH /repos/{owner}/{repo}/issues/{number}` with the installation's token; for GitLab,
`PUT /projects/{path}/issues/{number}` with the access token; for Forgejo, `PATCH /repos/{owner}/{repo}/issues/{number}`
with the access token), and "blocked by #n" in the
new body becomes the card's blockers, as the poll reads them; without a forge the blockers stay as they are. Then the
card is stored, the decision is recorded (kind `edit_card`, choice `edit_card`, decider `person` or `agent:Moebius`,
the card's id; its context has `title`, `body`, `previous_title` and `previous_body`) and the event `card_updated` is
sent. A title and body that change nothing record nothing. Response: the `Card`. Errors: `404` no such card; `422` the
title is blank; `500` the forge refused the change, and then the card is as it was and nothing is recorded.

### `POST /v1/cards/{id}/approve`

A person approves an issue that waits in the backlog: the decision is recorded (kind `approve_issue`, decider `person`),
the card's inbox items are resolved, and the card moves to `Not started`.

Request: `{"reason": "..."}`; `reason` is optional.

Response: the `Card`. Errors: `404` no such card; `422` the card is not in `Backlog`.

### `POST /v1/changes/{id}/merge`

A person merges an open change: the decision is recorded (kind `merge`, decider `person`), the forge merges it (a
no-fast-forward merge into the default branch of the clone for a repository without a forge; the GitHub API for a pull
request; the GitLab API for a merge request, with the merge commit's message; the Forgejo API for a pull request, with a
merge commit titled with the message), the change becomes `merged`, its inbox items are resolved and its card, if it has one, moves to `Done`. A
change from a chat has no card: its `ready_to_merge` item is resolved and the commit message is `Merge <branch>`.

A change of the harness configuration repository (`harness_repository` true) is merged into the default branch of its
clone with a merge commit, which is pushed to `origin`; the agents are read again, so the next run uses what the change
brought. A push that `origin` refuses undoes the merge and is a `500`. The decision is recorded as for any change, with
the repository `harness`.

Request: `{"reason": "..."}`; `reason` is optional, but the body is not: send `{}`.

Response: `{"commit": "<sha of the merge commit>"}`. Errors: `404` no such change; `422` the change is not `open`; `409` the
branch does not merge cleanly; `500` the forge failed.

### `POST /v1/changes/{id}/reject`

A person rejects an open change whose card waits in `Ready to merge`: the decision is recorded (kind `reject`, decider
`person`), the card's `ready_to_merge` inbox item is resolved, and the card moves back to `In progress` with the reason as
its `note`, so that the next Coder round is told (without a reason the note says a person rejected the change). The change
stays open: the next round updates it, and on GitHub its pull request stays.

Request: `{"reason": "..."}`; `reason` is optional, but send `{}` without one.

Response: the `Card`. Errors: `404` no such change; `422` the change is not `open` or its card is not in `Ready to merge`.

### `GET /v1/changes/{id}/diff`

What a change changes: `git diff <default branch>...<branch>` in the repository's clone (the harness configuration
repository's for a change of that), as a unified diff.

Response: `{"diff": string, "truncated": boolean}`. `diff` has secrets removed and is empty when nothing changed or the
branch is gone; a diff over 2 000 000 bytes is cut and `truncated` is true. Errors: `404` no such change.

### `POST /v1/cards/{id}/retry`

A person retries a card that waits in the inbox as stuck or whose pull request was closed (it reached its round limit or its token budget, or a person
stopped its run): the decision is recorded (kind `retry`, decider `person`), the card's inbox items are resolved, its
waiting reason is cleared and its `retried_after` is set to its latest run, so that its `rounds`, `reviews` and `tokens`
(see [column scripts](column-scripts.md)) count from the runs after it. The board then starts work on the card again.

A card whose pull request was closed on the forge without a merge waits in `Ready to merge` with an open `change_closed`
inbox item. Retrying it does the same, and moves the card to `In progress` with the note `The pull request was closed
without a merge.`; the closed change stays closed and the next publish opens a new change and pull request.

Request: `{"reason": "..."}`; `reason` is optional, but send `{}` without one.

Response: the `Card`. Errors: `404` no such card; `422` the card has neither an open `stuck` nor an open `change_closed` inbox item.

### `GET /v1/runs/{id}`

One run, for the live view. Response: the `Run`. Errors: `404` no such run.

### `GET /v1/runs/{id}/events`

The stored events of a run, in order. A client loads a long run in pieces and then follows `GET /v1/events` from the last
`seq` it has: every `run_event` on the stream carries the `seq` the event was stored under, so the client drops those
it already has and sees every event exactly once.

Query: `after` (default 0) returns only events with a `seq` above it; `limit` (default 500, at most 5000, at least 1)
is the most it returns.

Response: an array of `{"seq": number, "kind": string, "data": object, "at": number}`, oldest first, where `kind` and
`data` are those of the `run_event` (below). Errors: `404` no such run.

### `GET /v1/cards/{id}/runs`

The card's runs, oldest first. Response: an array of `Run`. Errors: `404` no such card.

### `GET /v1/cards/{id}/history`

The card's timeline, oldest first (entries of the same second are in the order of the kinds below). Response: an array
of `{"at": number, "kind": string, "data": object}` with these kinds:

| Kind | When `at` is | `data` |
|---|---|---|
| `moved` | The card moved to another column. Moves from before moves were recorded are not in the history. | `{"from": column, "to": column, "note": string or null}`, the card's `note` when it moved |
| `run_started` | The run was stored. | `{"run": id, "agent": string, "purpose": string}` |
| `run_ended` | The run ended. | `{"run": id, "status": "succeeded" \| "failed" \| "stopped"}` |
| `decided` | A decision was recorded for the card. | `{"judgement": id, "kind": string, "choice": string, "decider": string, "reason": string, "outcome": string or null}` |
| `attempt_judged` | An attempt of the card was judged by its tests and checks. | `{"run": id, "model": string or null, "result": "passed" \| "failed", "output": string}` |
| `attempts_settled` | The card's attempts were settled: one picked, or none passed. | `{"runs": [id], "picked": id or null, "reason": string or null}`, the reason the picked attempt was kept |
| `inbox_added` | An inbox item about the card was added. | `{"item": id, "kind": string, "title": string}` |
| `inbox_resolved` | That item was resolved. | `{"item": id, "kind": string, "title": string}` |

Errors: `404` no such card.

### `GET /v1/judgements`

The recorded decisions (what a person or a decider chose at a decision point), newest first. Query: `card` returns only
those about that card, `run` only those about that run (a chat has no card, so this is how its tool calls' deciders are
found), `kind` only those of that kind (such as `retry`, `merge` or `tool_call`); `limit` (default 100, at most 1000, at
least 1) is the most it returns. A card or run that does not exist simply has none.

Response: an array of `{"id": number, "kind": string, "choice": string, "reason": string, "decider": string,
"outcome": string or null, "card": number or null, "run": number or null, "at": number}`. The blobs of the question's
context are not included. For a refused tool call that was replayed, `outcome` is the replay's sentence, `replayed: <how
it ended>; <what happened to the files>` (see [the replays](harness-configuration.md#the-replays)).

### `POST /v1/judgements/{id}/replay`

Runs the call of a refused `tool_call` judgement again in a throwaway copy of its worktree, now, at any hour, as the
schedule does inside `[trial_window]`. The call runs once; the copy is removed; the judgement's `outcome` becomes the
replay's sentence; the event `replayed` is sent. Request: no body. Response: the `Replay`:

```json
{"id": 1, "judgement": 12, "run": 9, "exit": 0, "timed_out": false, "output": "", "created": [],
 "changed": ["src/lib.rs"], "deleted": ["src/a.rs", "src/b.rs"], "at": 1790000000}
```

`exit` is the exit code, null when the call was killed (`timed_out` says whether for the time) or did not run (then
`output` says why). `output` is what the call printed, secrets removed, the last 4 KB. `created`, `changed` and
`deleted` are paths relative to the worktree. Errors: `404` no such judgement; `409` it was replayed already, or has an
outcome already; `422` it is not a refused `tool_call`, or its call is not reversible, or its run or repository is gone,
with the reason.

### `GET /v1/replays`

The stored replays, newest first, at most 1000. Query: `judgement` returns the replay of that judgement, `run` those of
that run's calls. Response: an array of `Replay`.

### `GET /v1/judgements/kinds`

What the store holds now, per kind of judgement, kinds in alphabetical order. Response: an array of `KindCounts`:
`{"kind": string, "count": number, "with_outcome": number, "by_person": number, "held_out": number}`. `count` is the
judgements of the kind, `with_outcome` those with an outcome, `by_person` those whose decider is `person`, `held_out`
those in the held-out tenth (see [the training export](training-export.md#the-split)).

### `POST /v1/exports`

Writes the training-set export now (see [the training export](training-export.md)) to `<data_dir>/exports/<id>/` and
records the person's decision to do so as a judgement of kind `export` (decider `person`, context: `export` the id,
`kinds` the kinds written, joined by commas, and `counts` the `KindCounts` as JSON text). Request: `{"kinds": [string]}`,
the kinds to export; without `kinds`, or with `null`, every kind. The body is required, `{}` is the smallest. Response:
the `Export`:

```json
{"id": 1, "at": 1790000000, "kinds": [{"kind": "merge", "count": 12, "with_outcome": 12, "by_person": 12, "held_out": 1}]}
```

`id` counts up from 1. The export holds the judgements there were when it read them, not the judgement of its own
decision. Errors: `422` a kind has no judgements (no file is written for any kind).

### `GET /v1/exports`

The exports made through the API, newest first. Response: an array of `Export`.

### `GET /v1/exports/{id}/{file}`

Downloads a file of export `id`, `{file}` being `<kind>.jsonl` (such as `tool_call.jsonl`), streamed, as `application/x-ndjson`. The body is the file as the
export wrote it. Errors: `404` no such export or no such file in it.

### `GET /v1/usage`

What the model calls used: tokens, calls, how long they took, and where they went. Query: `days` (a whole number from 1
to 365, default 1) is the window, counted in local days of the server, today included: `1` is since local midnight (the
same "today" the day budget counts), `7` the last seven days. Anything else is `422`. Tokens are input plus output tokens.

Response:

```json
{
  "today": "2026-10-28",
  "days_in_window": 7,
  "totals": {"tokens": 1840000, "calls": 1284, "cached_share": 0.61, "model_ms": 11520000, "day_budget": 8000000},
  "days": [{"date": "2026-10-27", "tokens": 2520000, "model": "claude-sonnet-5-5"}],
  "models": [{"model": "claude-sonnet-5-5", "tokens": 960000, "calls": 690, "input_tokens": 700000, "cached_tokens": 520000}],
  "agents": [{"agent": "Coder", "calls": 702, "tokens": 1120000, "input_tokens": 1000000, "cached_tokens": 610000, "model_ms": 7080000}],
  "activities": [{"activity": "bash", "model_ms": 2140000, "calls": 214, "output_tokens": 61000}],
  "tool_waits": [{"tool": "bash", "calls": 31, "seconds": 2460}],
  "classes": [{"class": "strong", "model": "claude-sonnet-5-5", "tokens": 420000, "calls": 300}],
  "tool_output": {"results": 96, "bytes_before": 3400000, "bytes_after": 920000},
  "lens": [{"phase": "deep", "tool": "cargo", "runs": 31, "skipped": 2, "ms": 412000, "shown": 9},
           {"phase": "lint", "tool": "ruff", "runs": 140, "skipped": 0, "ms": 5200, "shown": 17}]
}
```

- `today`: the server's date, `YYYY-MM-DD`; the last day of the heatmap.
- `totals` covers the window. `cached_share` is the cached input tokens divided by the input tokens (0 to 1, and 0
  when there was no input). `model_ms` is the time the model calls took, in milliseconds, each measured from the request
  to the end of the response or stream. `day_budget` is `day_tokens` of `moebius.toml`, or `null` without one.
- `days` does not depend on `days_in_window`: it holds each of the last 365 local days that had usage, oldest first, with
  the tokens of the day and its leading model (the one with the most tokens that day; the first by name on a tie).
  Days without usage are not listed.
- `models` holds each model used in the window, the one with the most tokens first: tokens, calls (one per model call),
  input tokens (part of the tokens) and cached tokens (part of the input tokens).
- `agents` holds each agent whose runs used tokens in the window, the one with the most tokens first: calls, tokens,
  input tokens, cached tokens and model time. Usage that belongs to no run is counted under the agent `No run`.
- `activities` holds what the model did with its time in the window, the activity with the most model time first: the
  `activity` is a tool's name, `answering` or `unknown`. A model call that asked for `n` tools gives each of them `1/n`
  of its duration and of its output tokens (the remainder of a division, whole milliseconds or tokens, goes to the first
  tools; a tool named twice by one call gets both shares). A call that asked for no tool is `answering`. `model_ms` and
  `output_tokens` are the sums of those shares; `calls` counts a call once for each distinct tool it named, so it can
  add up to more than the calls of the window. A call recorded before the router read tools is `unknown`; `unknown` is
  listed only when the window holds no other call. What the router reads from answers is in `router.md`.
- `tool_waits` holds the tools whose calls waited longest in the window, at most 10: for each tool name, the number of
  calls and the seconds between their `tool_execution_start` and `tool_execution_end` run events, summed. Events
  are stamped in whole seconds. A call that has not ended is not counted.
- `classes` holds the calls of the window whose request named a model class (see [the router](router.md#model-classes)):
  for each class and the model of it that answered, the tokens and the calls, by class name and then the model with
  the most tokens first. The same calls are counted under their model in `models` too.
- `tool_output` is what keeping tool outputs small saved in the window: how many results were made smaller (their
  `tool_output` run events) and their bytes before and after, summed. See `agents.<name>.tool_output` in
  [configuration](configuration.md).
- `lens` is where the time of the lens's checks goes (see [the lens](bridge-protocol.md#the-lens)): for each phase
  (`syntax`, `lint` after an edit, `deep` before the run settles) and tool in the window, the checks that `runs` and the
  ones `skipped`, the `ms` they took in all and how many problems they told the agent about (`shown`); the one that took
  most time first. It is the sum of the `lens_check` run events.

Errors: `422` `days` is not from 1 to 365.

### `GET /v1/runs/{id}/diff`

What a run changed, as a unified diff: while the run has a worktree, everything in it since the run started, uncommitted
work included; afterwards the run's branch (`moebius/run-<id>`, `moebius/chat-<id>` for a chat) against the commit the
run started from. Response: `{"diff": string, "truncated": boolean}`, as for [a change](#get-v1changesiddiff); the diff
is empty for a chat that has no repository. Errors: `404` no such run.

### `POST /v1/runs/{id}/steer`

A person sends a message to a running run: the harness passes it to the agent (Pi-Bolt's `steer` command; Claude Code
is given it as its next user message, which it answers before the run ends), the run
stores a `steered` event with `{"message": ...}` (secrets removed), and the decision is recorded (kind `steer`, decider
`person`, the message in its context).

Request: `{"message": "...", "context": "..."}`; `message` is required and must not be blank. `context` is optional and
works as in `POST /v1/chats`: the agent is told it before the message (secrets removed, at most 4 000 characters), and
the `said` event holds only the message.

Response: the `Run`. Errors: `404` no such run; `422` the message is blank or the run is not running.

### `POST /v1/runs/{id}/stop`

A person stops a running run: the decision is recorded (kind `stop`, decider `person`) and the harness asks the agent to
abort (Pi-Bolt's `abort` command; a Pi-Bolt that has not exited after 10 seconds is killed; Claude Code is killed at
once; the agent's own process group goes with it, so the dev servers and watchers it started end too). The run ends as
`stopped`, also when Pi-Bolt had to be killed,
which publishes `run_ended`. The card is not retried by itself: its script puts it in the inbox as stuck ("stopped by a
person"), and a person decides with retry.

Request: `{"reason": "..."}`; `reason` is optional, but send `{}` without one.

Response: the `Run`, as it was when the request was made (still `running`). Errors: `404` no such run; `422` the run is not
running.

A chat that is stopped, also while it waits, ends as `stopped`; what it left in its repository's worktree is committed
on its branch first.

### `GET /v1/runs/{id}/terminal`

A shell in the run's worktree, for the person to look around and run commands. It upgrades to a WebSocket (with the
token, like every route) and starts the person's shell (`$SHELL`, else `/bin/sh`) on a pseudo-terminal in the worktree
of the run or chat, with `TERM=xterm-256color`.

- Binary messages carry bytes both ways: from the client to the shell's input, from the shell's output to the client.
- A text message `{"type": "resize", "cols": N, "rows": M}` sets the size of the terminal in cells (the size is 80 by
  24 until then).
- When the shell exits the server closes the socket; when the socket closes the server kills the shell and every
  process of its session, background jobs included (on Windows, its process tree).
- A run has at most 4 terminals open at a time.
- It is a person's shell, so the guard (which checks the tool calls of agents) does not look at it, and nothing the
  person types or sees is stored. Opening one stores a `shell_opened` run event, `{"who": "person"}`.

The worktree exists while the run or chat is `starting`, `running` or `waiting`. Errors, answered before the upgrade:
`404` no such run; `422` the run has no worktree (it is over) or already has 4 terminals open.

### `GET /v1/chats/{id}/tui`

Pi-Bolt's own terminal interface for a chat, for the person who prefers it to the conversation view. It upgrades to a
WebSocket and works like [the shell of a run](#get-v1runsidterminal), with the same binary and resize messages, but the
program on the pseudo-terminal is `pi-bolt` in its interactive mode, in the chat's worktree, started as the chat's own
process is: the same agent, model, provider (the router, with the chat's run token), instructions, skills, MCP servers,
extensions and the Moebius bridge, so every tool call is still checked and recorded as a decision of the chat's run.
Only `--mode rpc` is left out.

The chat and its terminal are one conversation:

- Opening it ends the chat's own Pi (idle, as the chat waits; the chat stays `waiting`), together with every process its
  agent started (a dev server, a watcher), and starts the interactive one
  with `--continue` on the chat's own agent directory and Pi sessions (`runs/<id>/agent`, `runs/<id>/sessions`), so it
  continues the chat's session with everything said so far.
- While it is open, Moebius reads what Pi adds to the session file (the newest one in the chat's sessions, every half
  second) and records it as the chat's run events, in the shapes Pi's RPC records have, so the chat view, the Fleet and
  the event stream show it as it happens: the person's message as a `said` event followed by `agent_start`; each
  thinking and text part of an answer as a `message_update` with a `thinking_delta` or `text_delta`
  (`{"assistantMessageEvent": {"type", "contentIndex", "delta"}}`); each tool call as a `tool_execution_start`
  (`{"toolCallId", "toolName", "args"}`) and its result as a `tool_execution_end`
  (`{"toolCallId", "toolName", "result": {"content"}, "isError", "durationMs"}`); an answer that does not end in a tool
  call as `agent_end` (`{"messages": [the answer]}`, which also makes it the run's summary) and `agent_settled`
  (`{"aborted": false}`). So the chat's status follows the agent as it does in the chat: `running` while it answers,
  `waiting` while the terminal waits for the person. What the session held before the terminal opened is not recorded
  again, and secrets are removed as from every event. As the file is read every half second, the `tool_check` event of
  a call made in the terminal (recorded when the bridge asks) can come before the events of the message that made it.
- Closing it (the socket closes, or the program exits) ends Pi-Bolt and its whole session, background jobs included, and starts the chat's own Pi
  again with `--continue` on the same session, without a message; the chat is `waiting` and its next message continues
  the whole conversation, the terminal's part included. The terminal's place is given back only then, so a message sent
  as it closes is answered `409`, never lost.
- Ending or stopping the chat while the terminal is open closes the terminal, and the chat ends (committed and offered
  as when it ends otherwise) without its own Pi starting again.

A chat has one terminal open at a time. While it is open the chat takes no message: `POST /v1/chats/{id}/messages`
answers `409`, as what is said goes through the terminal. It opens only while the chat is `waiting`, that is, between
turns; while the agent works it is `409`. The opening is recorded as a `tui_opened` run event `{"who": "person"}`.

Errors, answered before the upgrade: `401`; `403` for the run token of an agent (the terminal stays with a person);
`404` no such run; `409` the chat is not waiting, or has its terminal open already; `422` the run is not a chat or it
has ended.

### `GET /v1/terminal`

A shell on the server itself, for the person to look at `moebius.toml` and the machine Moebius runs on. It works like
[the shell of a run](#get-v1runsidterminal), with the same messages, but starts in the directory of the server's
configuration file (`moebius.toml`), not in a worktree.

- At most 4 terminals are open on the server at a time.
- A run has no part in it, so opening one is stored as a judgement of kind `open_terminal` (decider `person`, context
  `directory`: the directory the shell started in), never what is typed or seen.

Errors, answered before the upgrade: `403` for the run token of an agent (the shell stays with a person); `422` when 4
terminals are open already or the directory is gone.

### `GET /v1/repositories/{id}/terminal`

A shell in a repository's clone on the server, for the person to run commands in the project (the project page's
Terminal tabs). It works like [the shell of a run](#get-v1runsidterminal), with the same messages, but starts in the
clone of repository `id`, not in a worktree.

- At most 4 terminals are open on one repository at a time.
- A run has no part in it, so opening one is stored as a judgement of kind `open_terminal` (decider `person`, context
  `directory`: the clone, and `repository`: its name), never what is typed or seen.
- The ports its programs listen on are listed by `GET /v1/ports` as a terminal's, with no run.

Errors, answered before the upgrade: `401`; `403` for the run token of an agent (the shell stays with a person); `404`
no such repository; `422` when 4 terminals are open on it already or its clone is gone.

### `GET /v1/ports`

The TCP ports that processes of runs, chats and terminals listen on, as an array of `{"port": integer, "pid": integer,
"process": string, "kind": "run" \| "chat" \| "terminal", "run": id or null}`, by port. A port is listed when a socket in
the listening state on `127.0.0.1`, `::1` or every address has a process that belongs to a run, a chat or a terminal: a
terminal's when the process, or one that started it, is a shell a person opened (`run` is the run or chat it was opened
on, null for `GET /v1/terminal`), else the run's or chat's when the process, or one that started it, works in its
worktree. `process` is the program's name. It is read from `/proc` (Linux); on any other system the list is empty.

### `GET /v1/ports/{port}/tunnel`

Carries one TCP connection to `127.0.0.1:<port>` on the server (`[::1]` when only that answers). It upgrades to a
WebSocket: every binary message the client sends is written to the connection and every byte the connection sends comes
back as a binary message; either side closing ends both. The server listens on no new interface: the desktop app listens
on its own machine and opens one tunnel for each connection made to it, with the same address and token as for every
other route, so it works over an SSH tunnel too. A person may open a tunnel to any port of the server's loopback address,
detected or asked for by hand.

Errors, answered before the upgrade: `401`; `403` for the run token of an agent; `422` for port 0; `502` when nothing
answers on the port within 5 seconds.

### `GET /v1/runners`

The machines that run harnesses over SSH, the `[[runners]]` of `moebius.toml` (see [configuration](configuration.md#runners)),
by name. Response: an array of `{"name": string, "host": string, "user": string or null, "max_runs": integer, "busy":
integer}`; `busy` is how many runs go on the machine now. Nothing secret (the key file) is included.

### `GET /v1/agents`

The agents the server can run, for choosing one to chat with: the Moebius agent first and the Grilling partner second,
then the others by name. A built-in agent takes its model from its own `[agents.<name>]` or from `[agents.Coder]`; with
neither its `model` is empty, and a chat with it runs on the `model` it is started with. Response: an array of `{"name": string, "model":
string, "protocol": "openai" or "anthropic", "thinking": string, "token_budget": integer, "harness": "pi-bolt" or
"claude-code", "built_in": boolean, "error": string}`; `harness` is the harness the agent runs on;
`thinking` and `token_budget` are the agent's, left out when its configuration does not set them; `built_in` is true for the Moebius
agent and the `Grilling-partner`, which Moebius brings itself. `error` is left out unless the agent's files in the [harness
configuration](harness-configuration.md) are broken: such an agent has an empty `model`, cannot be run, and `error` says
why (the built-in agents still run, on the Coder's model). Instructions, skills, extensions and anything secret are not
included.

### `GET /v1/agents/{name}`

An agent of the harness configuration repository, as its files say: `{"name": string, "settings": {"model", "protocol",
"thinking", "token_budget", "harness", "runner", "skills", "extensions", "mcp", "tool_output", "engineering_skills"}, "instructions": string}` (`thinking`,
`token_budget`, `harness` and `runner` are left out when not set; no `harness` is `pi-bolt`, no `runner` is the server itself; `tool_output` is left out when it is the defaults; `engineering_skills` is left out when it is `true`, and `false` gives the agent none of the [engineering skills](harness-configuration.md#engineering-skills) of the Coder, the Reviewer and the Grilling partner). The files hold no secrets and are returned in full. Errors: `404` no such agent; `422` its files
are broken (the message says how), or the server has no `[harness]` and its agents are in `moebius.toml`.

A built-in agent (`Moebius`, `Grilling-partner`, `Retro-analyst`, `Autoresearcher`) also has `"built_in":
{"instructions": string, "skills": [{"name": string, "description": string}], "engineering_skills": [...]}`: the instructions it is always given
before its own, `engineering_skills` the skills of the same shape it is given besides unless its `engineering_skills` is `false` (the Grilling partner's `grilling`, `to-tickets` and `moebius-autonomous`; empty for the others), and the built-in skills it always has before its own, which nobody can change. The Moebius agent's are
the `moebius` skill and the task skills `moebius-operate`, `moebius-configure`, `moebius-workflows`, `moebius-extensions`, `moebius-plugins` and `moebius-report`; the others have the `moebius` skill. A built-in agent is answered
without files (then `settings` and `instructions` are left out) and without `[harness]`. `PUT` saves only the agent's
files and answers without `built_in`.

### `PUT /v1/agents/{name}`

A person adds or changes an agent of the harness configuration repository. Request: `{"settings": {...}, "instructions":
string}`, the shape `GET` answers without `name`; `settings.model` is required and unknown keys are refused;
`instructions` may be left out. The edit is validated as a file read is and, when valid, written, committed on the
default branch and pushed to `origin`; the decision is recorded (kind `edit_agent`, decider `person`), and the agent is
used from then on. Response: the agent as `GET` answers it, with `commit`, the short id of the commit the edit made (left
out when the edit changed nothing and so made no commit). Errors: `422` an invalid name or settings (the message says
what), or no `[harness]`; `500` git failed, such as a push that `origin` refused, and then nothing changed (see [harness
configuration](harness-configuration.md)).

### `DELETE /v1/agents/{name}`

A person deletes an agent of the harness configuration repository, committed and pushed the same way (kind `edit_agent`).
Response: `null`. Errors: `404` no such agent; `422` no `[harness]`; `500` as for `PUT`.

### `GET /v1/rules`

The rules page: what the guard refuses and why. Response:

```json
{
  "patterns": [{"name": "no-force-push", "description": "git push --force rewrites history on the remote"}],
  "prose": {
    "source": "harness",
    "editable": true,
    "text": "## no-vendoring\nDo not vendor.\n",
    "rules": [{"name": "no-vendoring", "text": "Do not vendor."}],
    "error": null
  },
  "repositories": [{"repository": "demo", "repository_id": 1, "text": "...", "rules": [{"name": "...", "text": "..."}], "error": null}],
  "refusals": {"no-force-push": 2}
}
```

- `patterns`: the built-in rules in the order they are tested, each with its name and what it refuses (the reason a
  refusal gives).
- `prose`: the server's rules file as it is now. `source` is `harness` for the `rules.md` of the harness configuration
  repository, `file` for the `rules_file` of the configuration, and `null` when the server has neither (then `text` is
  empty and there are no rules). `editable` is true exactly when `source` is `harness`. `text` is the file's text (empty
  when it does not exist), `rules` the rules as the parser reads them. `error` is `null` unless the file cannot be used:
  it cannot be read, a `## ` line has no name, or a name is repeated (the repeat is dropped by the parser).
- `repositories`: every repository, in the order of their ids, with the `.moebius/rules.md` on its default branch,
  each with `repository` (its name), `repository_id` (to save the file with `PUT
  /v1/repositories/{id}/files/.moebius/rules.md`), its text (without the final new line), its rules and `error` as for `prose`. A repository without the file has
  an empty `text` and no rules, so that a client can add its first rule.
- `refusals`: for each rule name, the number of `tool_call` judgements in the last 7 days that refused a call because of
  it, from the reason `<rule>: <why>` of a built-in pattern or of a rule in prose. A refusal that names no rule (the
  teacher failing, no teacher, a rules file that cannot be read) is not counted. A rule with no refusals is left out.

### `PUT /v1/rules`

A person replaces the server's rules in prose: the `rules.md` of the harness configuration repository, or without
`[harness]` the server's `rules_file`. Request: `{"text": string}`, the whole `rules.md`. The text must be fit to be saved: a `## ` line must have a name and a name must not be given twice (text
before the first `## ` line is free). It is written to the file (a final new line is added if missing); with `[harness]`
it is committed on the default branch with the message `Update the rules` and pushed to `origin`. Then the decision is
recorded (kind `edit_rules`, decider `person`, the text in its context, and `file` for a `rules_file`). Every tool call reads the file afresh, so the next one is
checked against the new rules. Response: the same as `GET /v1/rules` after the change, with `commit`, the short id of the
commit (left out when the text changed nothing, and always for a `rules_file`). Errors: `422` the text is not fit to be saved (the message says which line);
`500` git failed, such as a push that `origin` refused, and then nothing changed and no decision was recorded (see
[harness configuration](harness-configuration.md)).

### `GET /v1/skills`

The skills an agent's `skills` can name, for an editor to offer: an array of `{"name": string, "description": string,
"built_in": boolean, "source": {"url": string, "commit": string}}`, the built-in skills first (`moebius`, then the task skills `moebius-operate`, `moebius-configure`, `moebius-workflows`, `moebius-extensions`, `moebius-plugins` and `moebius-report`, then `moebius-autonomous`), then the
engineering skills (`tdd`, `diagnosing-bugs`, `code-review`, `grilling`, `to-tickets`) with `built_in` true and a `source`, the repository they were copied from and the commit, unless the harness configuration repository has a skill of that name, then the
directories of `skills/` in the harness configuration repository by name; a directory with the name of a reserved built-in skill
is left out (one with the name of an engineering skill replaces it). `description` is the `description` of the frontmatter of the skill's `SKILL.md`, empty when there is
none. Without `[harness]` the array is empty.

### `GET /v1/skills/{name}`

A skill of the harness configuration repository, or a built-in skill: `{"name": string, "description":
string, "text": string, "files": [string], "source": {"url": string, "commit": string}}` (`source` only for an engineering skill that the repository does not replace). `text` is the `SKILL.md` after its frontmatter; `files` are the paths of the
skill's other files, relative to its directory, sorted (each is read with `GET /v1/skills/{name}/files/{*path}`). Errors: `404` no such skill;
`409` the server has no `[harness]`: skills and MCP servers are edited in a harness configuration repository.

### `PUT /v1/skills/{name}`

A person adds or changes a skill. Request: `{"description": string, "text": string}` (`text` may be left out);
unknown keys are refused. The name is made of letters, digits, `-` and `_`, and the description is one line and not
blank. It writes `skills/<name>/SKILL.md` with the frontmatter `name` and `description`, then `text`; the skill's other
files are left as they are. The edit is committed (`Add the skill <name>` or `Update the skill <name>`) on the default
branch and pushed to `origin`, the decision is recorded (kind `edit_skill`, decider `person`), and the agents are read
again. Response: the skill as `GET` answers it, with `commit`, the short id of the commit (left out when nothing
changed). Errors: `409` no `[harness]`, or the skill is built in (`moebius` or a task skill) and read only; `422` an invalid
name or description; `500` as for `PUT /v1/agents/{name}`.

### `DELETE /v1/skills/{name}`

A person deletes a skill and its directory, committed and pushed the same way (`Delete the skill <name>`, kind
`edit_skill`). Response: `null`. Errors: `404` no such skill; `409` no `[harness]`, the skill is built in, or an agent
names it (the message names the agents); `500` as for `PUT`.

### `GET /v1/skills/{name}/files/{*path}`

One of a skill's other files; `{*path}` is the rest of the URL, from the skill's directory
(`/v1/skills/pdf/files/scripts/fill.py`). Response: the shape `GET /v1/repositories/{id}/files/{*path}` answers,
`{"path": string, "size": number, "text": string}`, with `binary` or `too_large` instead of `text` for a file that is
not UTF-8 text or is larger than 1 MiB. A built-in skill's files are read too. Errors: `404` no such skill or file (a
directory is not one); `409` no `[harness]`; `422` the path is empty, is `SKILL.md`, is absolute, climbs out with `..`,
names `.git` (in any case, or as `.git.`, `.git `, `GIT~1`), or goes through a symbolic link.

### `PUT /v1/skills/{name}/files/{*path}`

A person, or the Moebius agent, writes one of a skill's other files, creating it and its directories when they are
new. Request: `{"text": string}`; unknown keys are refused. The skill must exist (its `SKILL.md` is written with `PUT
/v1/skills/{name}`). The file is committed (`Add <path> of the skill <name>` or `Update <path> of the skill <name>`) on
the default branch and pushed to `origin`, and the decision is recorded (kind `edit_skill`, its context `skill`, `file`
and `text`). Text that changes nothing makes no commit and no decision. Response: the skill as `GET /v1/skills/{name}`
answers it, with `commit` (left out when nothing changed). Errors: `404` no such skill; `409` no `[harness]`, or the
skill is built in; `422` the path, as for `GET`, or it is a directory; `500` as for `PUT /v1/agents/{name}`.

```sh
jq -Rs '{text: .}' < fill.py > body.json
"$MOEBIUS" api PUT /v1/skills/pdf/files/scripts/fill.py --body body.json
```

### `DELETE /v1/skills/{name}/files/{*path}`

A person, or the Moebius agent, deletes one of a skill's other files, and the directories it leaves empty, committed and
pushed the same way (`Delete <path> of the skill <name>`, kind `edit_skill`). Response: the skill as `PUT` answers it.
Errors: `404` no such skill or file; `409` no `[harness]`, or the skill is built in; `422` the path, as for `GET`; `500`
as for `PUT`.

### `GET /v1/mcp-servers`

The servers of `mcp.toml` in the harness configuration repository, by name, for an editor to offer: an array of
`{"name": string, "kind": "stdio" or "remote", "error": string}`. `error` is left out unless the server cannot be used
(then `kind` is `unknown` when the table is too broken to tell). Commands, arguments, environments, URLs and headers are
never included. The array is empty without `[harness]` or when `mcp.toml` cannot be read.

### `GET /v1/mcp-servers/{name}`

A server of `mcp.toml` as its table says: `{"name": string, "command": string, "args": [string], "env": {name:
string}}` for a stdio server, or `{"name": string, "url": string, "headers": {name: string}}` for a remote one; empty
`args`, `env` and `headers` are left out. A value holds its `${NAME}` references as written, never what they stand for.
Errors: `404` no such server; `409` no `[harness]`; `422` the server or `mcp.toml` cannot be used (the message says why).

### `PUT /v1/mcp-servers/{name}`

A person adds or changes a server. Request: the shape `GET` answers, without `name`. It is checked as reading `mcp.toml`
checks it (see [harness configuration](harness-configuration.md#mcptoml)): one of `command` and `url`, only the keys
that belong to it, and `${NAME}` only for a `NAME` in `mcp_env` of `[harness]`. The server's table in `mcp.toml` is
rewritten, keeping the file's other tables and comments, committed (`Add the MCP server <name>` or `Update the MCP
server <name>`) and pushed, the decision is recorded (kind `edit_mcp_server`, decider `person`), and the agents are read
again. Response: the server as `GET` answers it, with `commit`. Errors: `409` no `[harness]`; `422` the server would not
read, with the reason; `500` as for `PUT /v1/agents/{name}`.

### `DELETE /v1/mcp-servers/{name}`

A person deletes a server from `mcp.toml`, committed and pushed the same way (`Delete the MCP server <name>`, kind
`edit_mcp_server`). Response: `null`. Errors: `404` no such server; `409` no `[harness]`, or an agent names the server
(the message names the agents); `500` as for `PUT`.

### `GET /v1/extensions/search`

The packages of the catalog of pi.dev: the npm registry's search (`[packages] npm_registry`) for the keyword `pi-package`
and the words of `q`, at most 40. Query: `q` (words; none finds the most relevant packages) and `type` (`extension`,
`skill`, `theme` or `prompt`; only packages whose keywords, with or without the prefix `pi-`, name it). Response: an
array of `{"name", "version", "description", "kinds": [string], "weekly_downloads": number or null, "npm", "homepage",
"repository", "date"}`; the links and date are null when the registry gives none. Allowed to a person and to the Moebius
agent. Errors: `422` an unknown `type`; `422` the registry cannot be reached or answers something else.

### `GET /v1/extensions/info`

What the package of a source is, fetched without its dependencies into a throwaway directory. Query: `source`, such as
`npm:pi-web-access`, `npm:@scope/name@^1.2.0` or `git:github.com/owner/repo@v1`. Response: `{"name", "source", "version",
"resolved", "integrity" (null for git), "description", "resources": {"extensions", "skills", "prompts", "themes"}
(paths in the package), "dependencies": [string], "install_scripts": [string] (scripts npm would run and Moebius does
not), "files": number, "installed": the installed version or null}`. Errors: `404` no such package, version or ref;
`422` the source is not one, or the download does not match its integrity.

### `GET /v1/extensions/packages`

The packages the harness configuration repository vendors (`packages.lock`), by name: an array of `{"name", "source",
"version", "resolved", "integrity", "sha256", "resources", "install_scripts", "agents": [string] (the agents that name a
resource of it), "error": string or null}`. `error` says why the files cannot be read. Errors: `409` no `[harness]`.

### `POST /v1/extensions/packages`

Proposes installing a package. Request: `{"source": string, "agent": string}`; `agent` is optional and names an agent
that is given the package's extensions and skills as `pkg:<name>` in the same change. The package is fetched (an npm
download whose SHA-512 is not the registry's `dist.integrity` is refused; no install script runs), put on a branch
`moebius/ext-add-<name>-<n>` of the harness configuration clone, checked as a chat's branch is, pushed and offered as a
change of the harness configuration repository with a `ready_to_merge` inbox item (see [Packages](harness-configuration.md#packages)).
Nothing is installed until a person merges the change (`POST /v1/changes/{id}/merge`). Allowed to a person and to the
Moebius agent; the agent's token cannot merge it. Response: `{"change": Change, "package", "action": "add", "source",
"from": null, "to": version, "resolved", "integrity", "resources", "agents", "changelog": "", "files": ["A packages/..."],
"dependency_files": number, "diff": string}`. `files` are the files of the package, without what npm installed below
`node_modules`, which `dependency_files` counts. Errors: `404` no such package, version or ref; `409` the package is
installed already, or no `[harness]`; `422` the source or version is not valid, the integrity does not match, npm or the
registry failed, the agent does not exist or the package has nothing to give it, or the branch is not fit.

### `POST /v1/extensions/packages/{name}/update`

Proposes updating one package to the newest version its source allows: for npm the newest release within the range that
was chosen when it was installed (the `latest` tag for none), for git the newest commit of the ref (a commit is pinned).
`{name}` is the package's name, with a `/` written `%2F`. Request body: `{}`. Response: `{"offered": [Offered], "up_to_date":
[name], "failed": [{"package", "error"}]}`, where `Offered` is the response of `POST /v1/extensions/packages` with
`"action": "update"`, `from` and `to` the versions, `changelog` the sections of the package's changelog (`CHANGELOG.md`,
or `pi.changelogPath`) after `from` up to `to` (headings `## [x.y.z]` or `## x.y.z - date`), `files` the changed files
as `A`, `M` or `D` and a path, and `diff` the diff of them (cut off at 100,000 characters). The open `extension_update`
inbox item of the package is resolved. Errors: `404` no such package; for one package asked alone, the other errors of
`POST /v1/extensions/packages` instead of `failed`.

### `POST /v1/extensions/update`

Proposes updating every installed package, as the route above does for one: the response has an `offered` entry for each
package with a newer version, `up_to_date` for the others, and `failed` for those whose newer version could not be fetched.

### `DELETE /v1/extensions/packages/{name}`

Proposes removing a package: its directory, its entry in `packages.lock` and its names out of the agents that have them,
on a branch `moebius/ext-remove-<name>-<n>` offered as a change. Response: `Offered` with `"action": "remove"`, `from`
the version and `agents` the agents that lose a name. Errors: `404` no such package; `409` no `[harness]`.

A change that adds, updates or removes a package is merged by a person only: `POST /v1/changes/{id}/merge` (and `POST
/v1/proposals/{id}/merge` for a proposal of the Retro) answers `403` with `only a person can merge a change that
installs, updates or removes a package` to a run token when the change touches `packages/` or `packages.lock`. An
extension is code that runs in every Pi process of the agents that name it. The check is made again, on the commit that
is merged, when the branch is merged, so a commit added to the branch in between does not get past it. The `agent` of
`POST /v1/extensions/packages` must be an agent name (`422` otherwise).

### `POST /v1/uploads`

Stores a zip file on the server, for the Moebius agent to import skills or MCP servers from (the desktop app's Import
button). Request: the file itself as the body, with `Content-Type: application/zip`, at most 20 MiB (20,971,520
bytes). The file is written to `<data_dir>/uploads/<id>.zip`, where `id` is 32 random hex digits, and kept until
someone removes it. Response: `{"id": string, "path": string}`, `path` being where the file is on the server. Allowed to
a person and to the Moebius agent's run token. Errors: `413` the body is larger than the limit; `415` another content
type; `422` an empty body; `500` the file cannot be written.

```sh
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/zip' --data-binary @skills.zip \
  http://127.0.0.1:7171/v1/uploads
```

### `GET /v1/models`

The models the server uses and where it reaches them. Never a key: an upstream or the decision model says only whether it
has one. Response:

```json
{
  "upstreams": [{"protocol": "anthropic", "base_url": "https://api.anthropic.com", "api_key_set": true}],
  "agents": [{"name": "Coder", "model": "qwen3.8-flash-next", "protocol": "openai", "thinking": "medium", "token_budget": 2000000, "built_in": false}],
  "teacher": {"model": "qwen3.8-flash-next", "protocol": "openai", "timeout_seconds": 20},
  "decision_model": {"url": "http://127.0.0.1:8081/v1", "model": "moebius-decider", "timeout_seconds": 20, "api_key_set": false}
}
```

- `upstreams` are the `[[router.upstreams]]` the server runs with, in the order of the configuration. `api_key_set` is
  true when the upstream has an `api_key_env` or an `api_key_secret`; the server does not start when that variable is not
  set or no key is stored under that name. Neither the key nor the name of the variable or the secret is shown. The configuration names no models an upstream serves, so none are listed.
- `oauth` is `null`, or for an upstream that logs in with OAuth (see [the router](router.md#oauth-logins))
  `{"flow": "device" | "code", "logged_in": boolean, "warning": string, "preset": string | null, "needs": string | null}`
  (`preset` the built-in provider, `needs` what the upstream needs that the router does not speak yet, so that its
  requests fail): `logged_in` is true when a refresh token is
  stored for it (whether it still holds, `GET /v1/models/upstreams/{place}/login` tells), and `warning` the text the
  person confirms before a login starts.
- `agents` are the agents of [`GET /v1/agents`](#get-v1agents), with the same fields.
- `teacher` and `decision_model` are `null` without a `[teacher]` or `[decision_model]` table.
- A user name and password in a URL are removed from `base_url` and `url`.

### `POST /v1/models/upstreams/{place}/login`

A person starts the OAuth login of the upstream at `place` (from 0, in the order of `[[router.upstreams]]`). Request:
`{"accept_terms": true}`, which says the person confirmed the upstream's `warning` (that using a subscription through a
third-party tool may break the provider's terms). The login goes on in the background (see
[the router](router.md#oauth-logins)); a login started before for the same upstream is replaced. The confirmation is
recorded as a decision (kind `oauth_login`, decider `person`, the upstream's URL and the warning in its context).
Response, a login:

```json
{"state": "waiting", "flow": "device", "user_code": "WDJB-MJHT", "verification_uri": "https://login.example.com/device",
 "verification_uri_complete": null, "authorize_url": null, "message": "Open https://login.example.com/device and enter the code WDJB-MJHT."}
```

- `state` is `logged_out`, `waiting`, `logged_in` or `failed`.
- For `device`, `user_code` is entered at `verification_uri`; `verification_uri_complete`, when the provider gives it,
  is the page with the code filled in. For `code`, `authorize_url` is the provider's page, which the app opens in the
  browser. The others are `null`.
- `message` says in plain words where the login stands. No token is ever in a login.

Errors: `403` an agent's token; `404` there is no upstream at `place`; `422` `accept_terms` is not `true`, or the
upstream has no `oauth` table; `502` the provider's device URL refused or could not be reached.

### `POST /v1/models/upstreams/{place}/login/code`

A person finishes the waiting `code` login of the upstream at `place` with what the browser showed. Request:
`{"pasted": string}`: the code, `code#state`, or the whole address the browser ended on (see
[the router](router.md#oauth-logins)). Response: the login as it stands (`waiting`; the exchange of the code goes on in
the background and `GET` below tells how it ended). Errors: `403` an agent's token; `404` no upstream; `409` the upstream
has no login waiting for a code (none started, a `device` login, or one that ended); `422` nothing in `pasted` is a code,
or its `state` is not the waiting login's (the login goes on waiting).

### `GET /v1/models/providers`

The providers built in, which an upstream's `preset` names, in the order the Models page offers them: `[{"name":
"anthropic", "title": "Anthropic (Claude Pro/Max)", "protocol": "anthropic", "base_url": "https://api.anthropic.com",
"needs": null}]`. `needs`, when not `null`, is what the provider's upstream needs that the router does not speak yet (every built-in provider is served now, so it is `null`). The
app adds an upstream for a provider with `PUT /v1/settings` (`router.upstreams.<n>`: `protocol`, `base_url` and
`oauth.preset`), which takes effect without a restart.

### `GET /v1/models/upstreams/{place}/login`

Where the OAuth login of the upstream at `place` stands, as a login (above). While a started login waits, or after it
failed, that login is answered. Otherwise the server checks whether the stored login holds: `logged_out` when no refresh
token is stored or the provider refuses it, `logged_in` when the upstream lists its models with the access token
(`Logged in: the upstream lists 2 models.`), `failed` when it does not. The desktop app asks every second while a login
waits. Errors: `404` and `422` as above.

### `GET /v1/models/available`

The models the router's upstreams serve, for the pickers of the Models and Agents screens. The server asks every
upstream at the same time, each with a limit of 10 seconds, with the key it sends in the router (see
[the router](router.md#listing-models)). Query: `refresh=true` asks the upstreams again; without it the last answer is
returned if it is less than 5 minutes old. `decision=true` also asks the endpoint of `[decision_model]`, never kept, for
the Models screen's decision model. Response:

```json
{
  "models": [{"id": "claude-sonnet-4-5", "protocol": "anthropic", "upstream": "https://api.anthropic.com"}],
  "errors": [{"upstream": "https://api.example.com", "message": "401"}],
  "classes": [{"name": "strong", "models": ["claude-sonnet-4-5", "qwen-local"], "protocols": ["anthropic"]}],
  "decision_models": ["moebius-decider"]
}
```

- `models` are sorted by `protocol`, then `id`; the same model of the same upstream is listed once.
- `classes` are the model classes of `[router.classes]` by name (see [the router](router.md#model-classes)), which may be
  named wherever a model is: each with its models, first tried first, and the protocols `models` lists any of them
  under (empty when none is listed). They are the router's as the request is answered, never kept.
- `decision_models` are the ids the decision model's endpoint lists at `<url>/models`, with its key; empty without
  `decision=true` or without a decision model. An endpoint that cannot be asked, or whose key is not set, is in
  `errors` under its `url`.
- An upstream that cannot be asked is in `errors` with a short `message`: an HTTP status such as `401`, `timed out`,
  `could not connect`, `the answer is not JSON` or `the answer has no model list`. It does not fail the request.
- No key is in the answer. A user name and password in an upstream's URL are removed.

### `GET /v1/start`

Whether the server is ready for a first card, for the Get started screen and `moebius doctor`. The steps are always the
same five, in the order a person takes them. Response:

```json
{
  "done": false,
  "steps": [
    {"key": "upstream", "title": "Connect a model provider", "done": true, "detail": "3 models are available."},
    {"key": "pi_bolt", "title": "Install Pi-Bolt", "done": false, "detail": "Could not run pi-bolt: it was not found. Set the program on the Settings page."},
    {"key": "harness", "title": "Keep agents, skills and MCP servers in a repository", "done": true, "detail": "Agents, skills, MCP servers and rules come from /srv/moebius/harness."},
    {"key": "coder", "title": "Choose the Coder's model", "done": true, "detail": "The Coder uses qwen."},
    {"key": "repository", "title": "Add a repository", "done": false, "detail": "No repository is added yet."}
  ],
  "warnings": []
}
```

- `done` is true when every step is.
- `upstream` is done when the router's upstreams list at least one model (the list `GET /v1/models/available` answers,
  without asking again if it is less than 5 minutes old). Without an upstream the detail is `Add an upstream on the
  Models page.`; when every upstream fails, it names each one as `Could not ask <url>: <message>.`
- `pi_bolt` is done when `pi_bolt.program` exists and `<program> --version` exits with status 0 within 10 seconds.
- `harness` is done when the server runs with `[harness]` (see [harness configuration](harness-configuration.md)), and
  the detail names its URL. Without it the detail says what `POST /v1/harness/setup` does, which the desktop app's
  "Set up" button sends.
- `coder` is done when the agent the default workflow starts in the code column, `Coder`, has a model.
- `repository` is done when at least one repository is added.
- `detail` is a sentence in plain words. It holds no key and no URL credentials.
- `warnings` are sentences the person should read even when every step is done: one for each upstream that logs in with
  OAuth, `<url> logs in with OAuth.` followed by the warning about the provider's terms. `moebius doctor` writes them
  after the steps.

### `GET /v1/plugins`

The plugins the plugin host has loaded, which answer the decisions (see [the plugin API](plugin-api.md)). Response:

```json
{
  "time_limit_seconds": 30,
  "plugins": [
    {"name": "guard", "version": "0.1.0", "kinds": ["decider"], "decides": ["tool_call"], "process": null},
    {"name": "allow-reads", "version": "0.1.0", "kinds": ["decider"], "decides": ["start_card"],
     "process": {"program": "/srv/plugins/allow-reads", "state": "running", "restarts": 0, "error": null,
                 "log": "/srv/.moebius/plugins/allow-reads.log"}}
  ]
}
```

`time_limit_seconds` is how long the host lets a call into any plugin take before it counts as refused. For each plugin,
by name, `kinds` are the kinds of its manifest (`forge`, `harness`, `runner`, `decider`, `router`, `view`) and `decides`
the kinds of decision it answers now, in alphabetical order: those routed to it, and those the decision model took over
with a merged takeover.

`process` is `null` for a plugin compiled in. For a plugin in its own process (`[[plugins]]` in
[the configuration](configuration.md#plugins)) it holds the `program` that runs; its `state`, one of `starting`,
`running` (it takes calls), `restarting` (it stopped and waits to be started again; calls are refused) and `stopped`;
how many times it was started again (`restarts`); why it stopped the last time (`error`, with secrets redacted, or
`null`); and the file its standard error goes to (`log`). Until such a plugin has sent its manifest, its `version` is
empty and its `kinds` are empty.

### Packages

The package manager (see [packages](packages.md)). A package is a `plugin`, `workflow`, `rules` or `skills` package.

#### `GET /v1/packages/search`

Query `q`: text; the packages whose name, description or kind contains it (case does not matter), all without one.
Every registry in [`[[registries]]`](configuration.md#registries) is read afresh (cloned or fetched). Response:

```json
{
  "packages": [
    {"name": "gate", "kind": "plugin", "version": "1.1.0", "description": "Holds cards marked wip.",
     "registry": "moebius-plugins", "compatible": true, "installed": null}
  ],
  "problems": [{"registry": "acme", "message": "cannot read index.toml: ..."}]
}
```

One package for each name and registry. `version` is the newest that works with this Moebius, or the newest of all with
`compatible` false when none does. `installed` is the installed version or null. A registry that cannot be cloned or
whose index is not valid is in `problems` (secrets redacted), and the others are answered.

#### `GET /v1/packages`

What is installed: an array of the lock's entries (`name`, `version`, `kind`, `registry`, `url`, `sha256`, and
`program`, `repository`, `change`, `files`, `rules` where they apply; see [the lock](packages.md#the-lock-data_dirpackageslock))
with `update_available`: the newest version the nightly check found, or null. Nothing is fetched.

#### `GET /v1/packages/{name}`

Every version of the package in the first registry that has it, newest first. Response: `{"name", "registry",
"installed": the lock's entry or null, "versions": [...]}`; each version is the index entry (`version`, `kind`,
`description`, `license`, `homepage`, `moebius`, `targets`, `plugin` with its `settings`, `downloads`, `files`) with
`compatible`, whether it works with this Moebius. Errors: `404` no registry has it; `502` a registry could not be read
and none has it.

#### `POST /v1/packages`

Installs a package (see [installing](packages.md#installing)). Request: `{"name", "version"?, "registry"?, "repository"?}`;
`version` is exact, without it the newest that works with this Moebius; `repository` is a repository's name, for a
workflow or rules package. Response, here and for the two routes below, an `Outcome`: `name`, `version`, `kind`,
`action` (`installed`, `updated`, `removed` or `unchanged`), `restart_needed` (a launcher is read when the server
starts), `change` (the change that proposes the files of a workflow, rules or skills package; absent for a plugin) and
`plugin` (a plugin's `program`, `state`, `restarts` and `error`, as `GET /v1/plugins` has them; absent for the others) and `leftover` (after removing a plugin, the folder of its
files when it could not be deleted; the removal is done all the same; absent otherwise).
Errors: `403` a run token asks for a plugin (the Moebius agent may propose the other kinds); `404` no registry has the
package; `409` it is installed; `422` the version does not exist or does not work with this Moebius, no download fits
this target or the sha256 differs, there is no such registry or repository, or the files cannot be proposed (skills
without a harness configuration repository); `502` a registry or a download could not be reached. Recorded as the
decision `install_package`.

#### `POST /v1/packages/{name}/update`

Updates to the newest version of the package's registry that works with this Moebius; `unchanged` when it has it.
Request body: none. Person only (`403` for a run token). Errors: `404` it is not installed; `422` the new version does not
start (the old one stays) or is refused as an install is. Recorded as `update_package`.

#### `DELETE /v1/packages/{name}`

Removes the package. Person only. Error: `404` it is not installed. Recorded as `remove_package`.

### `GET /v1/settings`

Every key of [`moebius.toml`](configuration.md) with its value in the file, or its default, for the Settings and
Models pages to build their forms from. The file is read afresh, so a change made by hand shows. Response:

```json
{
  "file": "/srv/moebius/moebius.toml",
  "values": {
    "listen": {"value": "127.0.0.1:7171", "default": true},
    "board.round_limit": {"value": 7, "default": false},
    "budgets.day_tokens": {"value": null, "default": true},
    "router.upstreams": {"value": [{"protocol": "anthropic", "base_url": "https://api.anthropic.com", "api_key_secret": "anthropic"}], "default": false},
    "agents": {"value": {"Coder": {"model": "qwen3.8-flash-next", "token_budget": 2000000}}, "default": false}
  },
  "secrets": ["anthropic"],
  "restart_needed": false,
  "restart_keys": []
}
```

- `file` is the absolute path of the configuration file.
- `values` has an entry for every key of the reference, by its dotted name (`board.round_limit`; `harness.mcp_env`;
  `decision_model.shadow`), with the whole `router.upstreams` (an array of objects) and the whole `agents` (an object of
  agents by name). `value` is what the file says, as JSON, or the default when `default` is true; `null` when the key
  has no default and the file does not set it. Paths are as written in the file.
- `secrets` are the names of the stored API keys, never the keys. An upstream or the decision model names one with
  `api_key_secret`, and an environment variable with `api_key_env`; neither the variable's value nor a key is ever
  in an answer. A user name and password in `harness.url`, `decision_model.url` and the upstreams' `base_url` are
  removed.
- `restart_keys` are the keys whose value in the file is not the one the running server started with and that the
  server uses only after it restarts (see [Editing the file from the app](configuration.md#editing-the-file-from-the-app));
  `restart_needed` is true when there are any.

### `PUT /v1/settings`

A person changes `moebius.toml`. Request: `{"set": {...}, "remove": [...]}`, both optional.

- `set` maps a dotted key to its new value: a JSON string, number, boolean or array of them for a key, an object for a
  table (`"teacher": {"model": "m"}` replaces the table, `"agents.Coder": {...}` replaces that agent), an array of
  objects for the router's upstreams (`"router.upstreams": [...]` replaces them all). One upstream is addressed by its
  position: `router.upstreams.0` replaces the first, the position after the last adds one. A key or table that is not
  in the file is added. A value is never `null`: remove the key instead.
- `remove` lists dotted keys or tables to take out (`budgets.day_tokens`, `teacher`, `agents.Coder`,
  `router.upstreams.1`); one that is not in the file is ignored. Removals are made before the sets.

The change is made to the text of the file, so its comments and the order of its keys stay. The result is read as
`moebius serve` would read it; if that fails (an unknown key, a value of the wrong type or out of range, an environment
variable of an upstream that is not set, a secret that is not stored, a GitHub App key file, a GitLab or Forgejo
token file that cannot be read, a token secret that is not stored or a token that is empty, `[agents.*]` together with
`[harness]`) the
answer is `422` with the message of the parse and nothing is written. Otherwise the file is written in one step (a
temporary file in its directory that replaces it), the decision is recorded (kind `edit_settings`, decider `person`,
the `set` and `remove` in its context; a URL has no user name and password there, and a key is never part of a
change), and the keys that are live take effect at once. Response: `{"restart_needed": boolean, "restart_keys":
[string]}`, as in `GET /v1/settings`: true with the keys that the running server uses only after it restarts. Errors:
`422` as above or a key that is not a key of the configuration; `500` the file cannot be read or written.

### `PUT /v1/secrets/{name}`

Stores an API key under `name` (letters, digits, `-` and `_`) in `secrets.toml` of the data directory, which only its
owner can read, replacing a key stored under the name before. Request: `{"value": string}`, not blank. The key is not
returned, stored anywhere else or logged. Response: the names of the stored keys. Errors: `422` the name or the key is
not allowed.

### `DELETE /v1/secrets/{name}`

Removes the key stored under `name`. Response: the names of the keys left. Errors: `404` there is none. A key that
`api_key_secret` still names is not checked: the next start fails until the name is stored again.

### `POST /v1/webhooks/{forge}`

A forge's webhook delivery (`github`, `gitlab` or `forgejo`; see [Webhooks](configuration.md#webhooks)). It needs no
token: the forge proves the delivery with `X-Hub-Signature-256` (GitHub), `X-Gitlab-Token` (GitLab) or `X-Gitea-Signature`
(Forgejo), checked in constant time against the secret `webhook-<forge>`. The body is JSON from the forge and is not
stored or used as data; it only names the repository to poll. Answers `202` with an empty body when the delivery is
verified, and the poll of the forge runs at once (for the repository `repository.full_name`, GitLab's
`project.path_with_namespace`, names, or for all of the forge's repositories when it names none). Errors: `401` the
signature is missing, malformed or wrong, and nothing changed; `404` the forge is unknown, is not set up, or has no
secret stored.

### `GET /v1/webhooks`

For each forge that is set up, its webhook as the Settings page shows it: `[{"forge": "github", "path":
"/v1/webhooks/github", "header": "X-Hub-Signature-256", "secret": string or null}]`. `path` is appended to the address the
forge reaches the server at; `header` is where the forge puts its proof; `secret` is the secret to set on the forge, null
until one is made or stored. This is the one answer that holds a secret, so only a person may read it (`403` to a run
token).

### `POST /v1/webhooks/{forge}/secret`

Makes a new random secret for the forge's webhook, stores it as `webhook-<forge>` and replaces the one before, which stops
working at once. No request body. Response: the forge's entry of `GET /v1/webhooks`. Errors: `403` to a run token; `404`
the forge is unknown or not set up.

### `POST /v1/server/restart`

Restarts the server. It answers `202` with an empty body, and a moment later stops the board, the schedule and the Pi
processes of the runs going (the next start fails those runs, as it does after any stop, so the board starts them
again) and starts the same program again with the same arguments, on the same configuration file. Clients lose their
connection and make it again. A server that is not the `moebius` binary only stops serving.

### `POST /v1/harness/setup`

A person moves the agents and rules of `moebius.toml` into a new harness configuration repository. No request body.
The server makes `harness/` next to `moebius.toml` a git repository on `main` and commits in it "Set up the harness
configuration repository": every `[agents.<name>]` as `agents/<name>.toml` and `agents/<name>.md`, each skill directory
an agent names copied to `skills/<directory name>/` and each extension file to `extensions/<file name>` (the agents'
`skills` and `extensions` name the copies), the rules file as `rules.md` (empty without one), an `mcp.toml` without
servers and a `README.md`; the agents are read back as the server reads them first (see [a repository on the
server](harness-configuration.md#a-repository-on-the-server)). It then writes `moebius.toml` in one step with `[harness]
url` set to the repository's absolute path and without `[agents.*]` and `rules_file`, its other keys and comments kept,
and records the decision (kind `setup_harness`, decider `person`, the file, the repository, the agents and where each
skill and extension was copied in its context). The file is saved as `PUT /v1/settings` saves a change, so the result
must read as `moebius serve` reads it and the keys that are live take effect at once; the running server is not
otherwise changed: it uses the repository once it restarts (`POST /v1/server/restart`, which the desktop app sends
next).

Response: `{"restart_needed": true, "restart_keys": ["harness.url"]}`, as `PUT /v1/settings` answers. Errors: `403` a
run token; `409` the file has `[harness]` already, or `harness/` exists and is not empty; `422` an agent cannot be moved
(a skill or extension path that is not there, a skill named like a built-in one, two skills or extensions with the same
name), or the file cannot be used as `moebius serve` reads it (it does not parse, an environment variable of an upstream
is not set, a secret is not stored); `500` a file cannot be read or written. Whatever fails before the file is written
leaves `moebius.toml` and `harness/` as they were; a write that fails leaves the repository the server made in
`harness/`.

### `POST /v1/chats`

Starts a chat: an interactive run without a card. The agent answers the first message as a stream of run events, and
the chat then waits (status `waiting`) for the next message. It works in a new worktree on the branch `moebius/chat-<run id>`
from the repository's default branch when `repository` is given, so that what the agent builds can be offered as a
change (see `POST /v1/chats/{id}/offer` and `POST /v1/chats/{id}/end`), or otherwise in an empty directory `chats/<run id>` under the data directory, which is removed when the
chat ends. The chat's run uses the agent's configuration like any run; its `token_budget` counts the chat's own tokens, the
daily budget applies, and a chat does not count against the board's run limit.

A chat lasts until it is ended or stopped. A chat that has waited 30 minutes for its next message, with no terminal open,
has its Pi ended to free what it holds, together with every process its agent started (a dev server, a watcher); the chat stays `waiting` with its worktree, and the next message starts its Pi
again with `--continue` on the chat's own session, so the agent answers it with the whole conversation. Ending, stopping
or opening the terminal of such a chat works as for any waiting chat.

Without `message` the chat's Pi starts with no prompt and the chat is `waiting` at once, so its terminal
(`GET /v1/chats/{id}/tui`) can be opened straight away; its first message, written later, gives it its title.

A chat runs on `model` and with the reasoning level `thinking` in place of its agent's when they are given: its Pi is
started with `--model` and `--thinking` of them every time (the first time, after a terminal, after waiting long), and
the router serves the model as for any run. The protocol to the model stays the agent's, unless the router's upstreams
list the model (see `GET /v1/models/available`) only under the other protocol: then the chat speaks that one. A
`model` that names a model class is listed under the protocols of its models. The
choice is stored as the chat's first `settings_changed` run event and listed with the chat. Pi's `models.json` marks
the model as one that reasons when the level is not `off`, as Pi otherwise turns any level off.

Request:

| Field | Type | Meaning |
|---|---|---|
| `agent` | string | An agent of the configuration, `Moebius` or `Grilling-partner`. Without it the chat is with the Moebius agent (see [configuration](configuration.md)). With `Grilling-partner` the chat is a grilling (see `POST /v1/grillings/{id}/confirm`) and `repository` is required. |
| `message` | string | The first message, not blank, sent to the agent as it is, with no issue prompt around it. Optional: without it the chat waits for its first message (required with `continues`). |
| `model` | string | A model to run the chat on in place of the agent's, not blank. Optional. |
| `thinking` | string | A reasoning level to run the chat with in place of the agent's: `off`, `minimal`, `low`, `medium`, `high`, `xhigh` or `max` (the levels Pi-Bolt takes). Optional. |
| `repository` | number | The id of a repository to work in. |
| `harness` | boolean | Work in the harness configuration repository instead (default false): in a worktree on the branch `moebius/chat-<run id>` of its default branch, so that the agents, skills, extensions, MCP servers and rules can be built as files and merged (see [changing it in a chat](harness-configuration.md#changing-it-in-a-chat)). The run has `harness_repository` true. Not together with `repository`. |
| `continues` | number | The id of an ended chat this one goes on from. Its agent, place (repository or harness configuration repository), model and reasoning level are used unless `agent`, `repository`, `model` or `thinking` name others, and the agent is given what was said in it (the person's messages and the agent's answers, at most the last 20 000 characters) before `message`. The new chat's `said` event holds only `message`. |
| `context` | string | Where the person is in the app (the page, the repository, the open card or chat, the text being edited), as the desktop app describes it. The agent is told it before `message`, under the line `[Where the person is in the Moebius app. Use it to understand what they mean; do not repeat it.]`, with secrets removed and cut to 4 000 characters; the `said` event holds only `message`. Optional; blank is the same as none. |
| `origin` | string | Where the chat is started: `chat` (the default), on the Chat page or by any client, `automation`, as an automation's chats are (see `POST /v1/automations`), or `assistant`, a conversation in the desktop app's floating window, which the person reads there. `GET /v1/chats` leaves out the chats of origin `assistant` unless asked for them, so the Chat page and its unread count do not show them; the run, its events and every other chat route work as for any chat. |

Response: the `Run` (`purpose` `chat`, `card_id` null). Errors: `404` no such repository or no run `continues`; `422`
the message is blank or missing with `continues`, `model` is blank, `thinking` is not one of the levels, the agent does not exist, it is a built-in agent and neither its own `[agents.<name>]` nor
`[agents.Coder]` is configured (the message says so), it is the Grilling partner and there is no `repository`, or
`continues` is not a chat or has not ended (write to it with `POST /v1/chats/{id}/messages`), `origin` is none of `chat`,
`assistant` and `automation`, `harness` is set together with `repository`, or `harness` is set and the server has no `[harness]`; `500` the worktree cannot be made.

### `GET /v1/chats`

The chats, newest first, without those of origin `assistant` (the desktop app's floating window). Query: `all=true`
lists those too, each with its `origin`; the desktop app asks for them so that its Fleet page shows the agents they
keep working and "Open in Chat" can show one. Response: an array of `Chat`. A chat's history is its run's events, read with
`GET /v1/runs/{id}/events` and followed on `GET /v1/events`.

### `POST /v1/chats/{id}/messages`

A person writes to a chat. The message is stored as a `said` run event (secrets removed) and sent to the agent as a
prompt: when the agent is waiting it answers at once; when it is working the message is delivered when it is done with
the current turn (a follow-up). To change what a working agent is doing, steer it with `POST /v1/runs/{id}/steer`.

A chat that ended because its agent went away (`status` `stopped`) is taken up again by the message, so that a person
carries on in the chat they are reading rather than starting a new one: the same `Run` is no longer ended (`ended_at`
null, `status` `running`), its worktree is made again on the branch it already has (for a chat with no repository, its
empty directory is simply made again), and its agent starts on a prompt that holds what was said in the chat before — at
most the last 20 000 characters of it — and then the message. A chat that ended any other
way (`succeeded`, `failed`) is ended for good and is continued by a new chat (`POST /v1/chats` with `continues`).

Request: `{"message": "...", "context": "..."}`; `message` is required and must not be blank. `context` is optional and
works as in `POST /v1/chats`: the agent is told it before the message (secrets removed, at most 4 000 characters), and
the `said` event holds only the message.

Response: the `Run`. Errors: `404` no such run; `422` the run is not a chat, the chat has ended for good, its agent
cannot start again, it is the Grilling partner (its issues are already read), or the message is blank.

The chat is `running` from the request on, before the agent says it has started, until the agent has answered.

### `POST /v1/chats/{id}/settings`

Changes the model or the reasoning level a chat that has not ended runs with, in place of its agent's (see
`POST /v1/chats`): `{"model": "...", "thinking": "..."}`, each optional and kept as it was when not given. The agent
answers the next message with them. The chat must be `waiting`, between turns: while its agent works the answer is
`409`.

The change is stored as a `settings_changed` run event holding what the chat now runs with, `{"model", "protocol",
"thinking"}` (each null while the agent's is used; `protocol` is `openai` or `anthropic`, chosen as in
`POST /v1/chats`), and the latest such event counts. Then the chat's Pi is told:

- When only the reasoning level changed and the Pi's model reasons (its level was not `off`, nor is the new one), Pi is
  sent `{"type": "set_thinking_level", "level"}` on its RPC channel, and its answer is recorded as a `response` event.
- When the model changed (Pi's `set_model` knows only the models of its `models.json`, which holds the one it was
  started with), or the level turns reasoning on or off, the chat's Pi is ended and started again with `--continue`
  on the same session and the new `--model` and `--thinking`; the chat stays `waiting` and keeps its conversation.
- A chat whose Pi was ended after waiting long, or whose terminal is open, has no Pi to tell: its Pi is started with
  the new settings when the next message comes or the terminal closes.

Response: the `Run`. Errors: `404` no such run; `409` the chat's agent is working; `422` the run is not a chat, the chat
has ended, `model` is blank or `thinking` is not one of `off`, `minimal`, `low`, `medium`, `high`, `xhigh` and `max`.

### `POST /v1/chats/{id}/offer`

Offers what the chat's worktree changes as a change, without ending the chat: what the agent left in the worktree is
committed on the chat's branch (message `Chat <id>`, "Committed by Moebius when the chat's work was offered."), and the
branch is offered exactly as when the chat ends (see `POST /v1/chats/{id}/end`): a change without a card on
`moebius/chat-<run id>`, published through the repository's forge, an open `ready_to_merge` inbox item with the chat's
title and the event `change_offered`; for a chat in the harness configuration repository, after the same check of the
branch. The chat goes on in the same worktree and takes messages as before.

Offered again after the chat has changed more, the chat's open change is updated rather than a second one made: its
`head` becomes the branch's new commit (on GitHub the branch is pushed to the same pull request), its inbox item is
resolved and a new one added, and `change_offered` is published. Offered again when the branch has not moved since, the
open change is the answer and nothing is made. A chat whose change was merged or closed offers a new change once it has
changed more. Ending the chat later offers what it changed since in the same way.

The offer is recorded as a decision of kind `offer_chat_changes` (decider `person`, or `agent:Moebius` for the Moebius
agent's run token), with the chat's run and the context `change` and `branch`.

Request: `{"reason": "..."}`, or `{}`; `reason` is optional.

Response: the `Change`. Errors: `404` no such run; `409` the chat's agent is working (`starting` or `running`), as its
worktree may be half edited: offer once it has answered; `422` the run is not a chat, the chat has ended, it is a
grilling, it works in no repository, it has nothing to offer (its branch changes nothing beyond the default branch, or
nothing that was not offered in a change since merged or closed), or its harness configuration branch fails the check
(the message says why).

### `POST /v1/chats/{id}/seen`

A person has read the chat up to the event `seq`: `{"seq": number}`. The chat's `seen` becomes that `seq`, unless it is
already further, since reading never goes back. The desktop app sends it while the chat is open on its Chat page, and
counts on the sidebar's Chat entry the chats that are unread. Response: `{}`. Errors: `404` no such run; `422` the run
is not a chat.

### `POST /v1/chats/{id}/title`

Names a chat: `{"title": "..."}`, at most 80 characters of it, taken after the spaces at either end. The name replaces
the title taken from the chat's first message everywhere the chat is titled: the list of chats, an inbox item and a
pull request made from the chat's branch. An empty or blank `title` gives that title back. A chat may be named at any
time, including after it ended: the name belongs to the chat, not to its run.

Response: the `Chat` as `GET /v1/chats` shows it, with its new `title`. Errors: `404` no such run; `422` the run is not
a chat.

### `POST /v1/chats/{id}/end`

The desktop app offers no way to end a chat (its work is offered with `POST /v1/chats/{id}/offer`, and a chat that waits
long frees its Pi by itself); the command line and the Moebius agent end chats with this route, and a grilling ends with
it. A person ends a chat that has not ended: Pi is asked to shut down (its standard input is closed; a Pi that has not exited
after 10 seconds is killed). With a repository, what the agent left in the worktree is committed on the chat's branch
(a chat that changed nothing has still succeeded), and then the worktree, or the empty directory, is removed.

If the chat's branch then has commits that the repository's default branch does not have, they are offered as a change
that has no card: a `Change` with `card_id` null on the branch `moebius/chat-<run id>`, published through the
repository's forge (on GitHub the branch is pushed and a pull request is opened whose title is the chat's title and
whose body says it came from the chat; without a forge there is nothing to publish), an open `ready_to_merge` inbox item
with the chat's title (`card_id` null, `data.change`, `data.branch`) and the event `change_offered`. A person merges it
with `POST /v1/changes/{id}/merge`, which resolves the item; the merge commit's message is `Merge <branch>`. A chat
whose work was offered with `POST /v1/chats/{id}/offer` updates its open change instead, and offers nothing new when it
changed nothing since. A chat that changed nothing, or has no repository, offers nothing, and a chat that ends as `failed` offers nothing. If the offer
fails, the chat's run gets a `moebius_error` event saying why and the work stays on the branch. A chat that is stopped
offers what it made in the same way.

A chat in the harness configuration repository (`harness` true) is committed the same way and offered as a change of
that repository: a `Change` with `repository_id` null and `harness_repository` true, no card and no pull request, on the
branch `moebius/chat-<run id>`, with the same `ready_to_merge` inbox item and `change_offered` event. Before it is
offered, the branch is checked as a branch of a Retro proposal is: it may change only harness configuration files
(`agents/`, `skills/`, `extensions/`, `rules.md`, `mcp.toml`), and every agent that could be read on the default branch
must still be readable with it. A branch that passes is pushed to `origin`. A branch that fails is not offered: the chat's
run gets a `moebius_error` event saying why, and the branch stays in the clone.

A grilling (a chat with `Grilling-partner`) commits nothing and offers no change: before the worktree is removed Moebius
reads what the agent left in it (see `POST /v1/grillings/{id}/confirm`) and stores a `Grilling`, whose status and id are
the event `grilling_ended`.

The chat ends as `succeeded`, which publishes `run_ended`; the change is offered before that.

Request: `{}`.

Response: the `Run`, as it was when the request was made. Errors: `404` no such run; `422` the run is not a chat or has
ended.

### `GET /v1/automations`

The automations, by name: an array of `Automation`.

### `POST /v1/automations`

Adds an automation: recurring work that runs on its own. Once a minute the server reads its local time; each automation
that is on and whose schedule came due since the minute before (or, after a late check, within the last five minutes)
fires: the server starts a chat as `POST /v1/chats` would, with the automation's agent, place and `prompt` as the first
message, and `origin` `automation`. A firing that came due while the server was not running is not made up. A run never
waits for a person: once the chat's agent has answered and the chat waits for a message, the next check ends it as
`POST /v1/chats/{id}/end` does, so what it changed is offered as a change and lands in the inbox. A firing due while the
chat of the automation's last started firing has neither ended nor answered is skipped and recorded as `skipped`. A chat
that cannot be started (its agent can no longer run, say) is recorded as `failed` with the reason. Each firing publishes
`automation_fired`.

Request:

| Field | Type | Meaning |
|---|---|---|
| `name` | string | Not blank. |
| `schedule` | string | A cron expression of five fields, minute (0-59), hour (0-23), day of the month (1-31), month (1-12 or `jan`-`dec`) and day of the week (0-7 or `sun`-`sat`, Sunday 0 or 7), in the server's local time. A field is `*`, a number, a range `a-b`, either with a step (`*/15`, `8-18/2`, `5/10` from 5 on), or a list of these separated by commas. When both the day of the month and the day of the week are restricted (neither starts with `*`), a day that matches either comes due, as in cron. |
| `agent` | string | An agent the server can run, as `GET /v1/agents` lists them. |
| `prompt` | string | The first message of each chat, not blank. Stored with secrets removed. |
| `repository` | number | The id of a repository its chats work in. Optional; without it and without `harness` each chat works in an empty directory. |
| `harness` | boolean | Its chats work in the harness configuration repository (default false). Not together with `repository`. |
| `enabled` | boolean | Whether the schedule fires it (default true). An automation that is off fires only with `POST /v1/automations/{id}/run`. |

Response: the `Automation`. Errors: `404` no such repository; `422` the name or prompt is blank, the schedule does not
parse (the message says which field is wrong), the agent does not exist, `harness` is set together with `repository`, or
`harness` is set and the server has no `[harness]`.

### `GET /v1/automations/{id}`

The `Automation`. Errors: `404` no such automation.

### `PUT /v1/automations/{id}`

Replaces an automation: the same body and checks as `POST /v1/automations`. Its firings stay, and it fires next when
its new schedule comes due. Response: the `Automation`. Errors: as `POST /v1/automations`, and `404` no such automation.

### `DELETE /v1/automations/{id}`

Deletes an automation and its firings; the chats it started stay. Response: `{}`. Errors: `404` no such automation.

### `POST /v1/automations/{id}/run`

Fires an automation now ("Run now"), whether it is on or not, as its schedule would: it is skipped while the chat of its
last started firing still runs. Request: `{}`. Response: the `Firing` (`manual` true). Errors: `404` no such automation.

### `GET /v1/automations/{id}/firings`

The automation's firings, newest first, at most 50: an array of `Firing`, each with its chat (`run_id`, whose events
are `GET /v1/runs/{id}/events`) and that chat's status. Errors: `404` no such automation.

### `POST /v1/retros`

Runs the Retro of a day now: the day's errors are collected into a report and, when the day had errors and `[harness]`
is configured, the Retro analyst starts (see [harness configuration](harness-configuration.md#the-retro)). The analysis
goes on after the response, which has the status `analysing`; its end is the event `retro_done`. Without `[harness]`, or
without errors, no run is made and the Retro is `reported` at once. The server does this by itself once a day, at
`retro_at` (see [configuration](configuration.md)).

Request: `{}` for today, or `{"date": "YYYY-MM-DD"}` for a local day. Response: the `RetroView`. Errors: `409` the day has
a Retro already; `422` the date is not a date.

`RetroView`: `date`, `status` (`reported`: the report is all there is; `analysing`: the analyst is running; `proposed`: it
ran and its proposals are stored, there may be none; `failed`: its run failed or was stopped, or the server stopped
meanwhile), `run_id` (the analyst's run, or null), `warnings` (array of strings: the branches left out and why, or why it
failed), `at`, `report` and `proposals` (array of `Proposal`).

`report`: `{"date", "errors": [...], "merged": [...]}`; `merged` are the cards that reached Done that day after one code
run and without being sent back from review, each `{"card", "title", "text"}` (`text` the start of the issue), which the
Retro analyst writes lessons from (see [lessons](#get-v1lessons)). A day with errors or merged cards starts the analyst.
`errors` are in the order in time. Each error is `{"kind", "run", "card", "agent", "text"}`;
`run`, `card` and `agent` are null where the error has none, and `text` is at most 500 characters, with secrets removed as
stored. The kinds are `tool_failed` (a tool call that ended in error: the tool and the start of what it said, not a call
Moebius refused), `tool_refused` (the tool and the rule's reason), `run_failed` (the run's last `moebius_error`),
`run_stopped`, `steered` (a person's message to a running run), and `checks_failed`, `conflict` and `review_findings`
(a card sent back from In review to In progress, with its note). The runs of the Retro analyst itself are left out.

`Proposal`: `id`, `kind`, `retro_date`, `branch` (`proposal/<date>-<name>`, also on the harness repository's `origin` for
a `harness` proposal), `title` (the subject of its commit), `body` (the rest of the message, without its `Cards:` line:
the Retro's reason), `agents` (the names of the agents whose files it changes), `cards` (the ids of the cards whose
errors it addresses, from the `Cards: 48, 50` line that ends its commit message; empty without one), `status`, `trials`
(array of `Trial`, oldest first), `diff`, `edits`, `repository`, `change` and `at`.

`kind` of a proposal: `harness` (a change to the harness configuration, on its branch), `settings` (a change to
`moebius.toml`) or `repository` (a change to a repository's own rules or column scripts in its `.moebius/`); see
[the Retro](harness-configuration.md#the-retro). For `settings` and `repository`, `diff` is a unified diff of what it
changes, `edits` is what it sets and removes (`{"set", "remove"}`, as `PUT /v1/settings` takes it) or the new text of
each file (`{"files": {".moebius/<file>": text}}`), `repository` is the repository's id for `repository` (else null),
and `change` is the id of the change a merged `repository` proposal was offered as (else null). A `harness` proposal has
an empty `diff`, `edits` `{}` and both null.

`status` of a proposal: `open` (not tried yet), `trying` (its trials are going on), `positive` (every trial was positive),
`negative` (some trial was), `untried` (no trial was negative, but some was skipped or failed, or it names no cards, so
nobody knows), `merged` or `dismissed`. A `positive` or `untried` proposal has an open inbox item of the kind `proposal`.
A `settings` or `repository` proposal is never tried: it is `untried` and in the inbox from the start.

`Trial`: `id`, `proposal_id`, `card_id` (the card it was tried on; null for the one trial of a proposal that names no
cards), `regression` (true for a card of the regression set, see
[the regression set](harness-configuration.md#the-regression-set); its `original` is then the card's code runs together,
with no `errors`, and its trial must pass where they did), `run_id` (the trial's run, or null when none was started), `outcome` (`positive`, `negative`, `skipped`: there was
nothing to try it on, or `failed`: it could not be tried), `reason` (text), `measurements` (an object, `{}` when
nothing was run) and `at`. `measurements` is `{"trial": {...}, "original": {...}}`: for the trial run `succeeded`,
`errors` (the kinds of the addressed errors that happened in the run), `checks` (true, false, or null when the repository
has no checks or the run made nothing to check), `tokens` and `rounds` (always 1); for the card's first code run `errors`
(the kinds of the card's errors in the report), `checks_failed` (the card was once sent back because the repository's
checks failed), `rounds` (the card's code runs) and `tokens`. See [the trials](harness-configuration.md#the-trials).

### `GET /v1/retros`

The Retros, the latest day first. Response: an array of `{"date", "status", "run_id", "errors" (how many the report
holds), "proposals" (how many), "at"}`.

### `GET /v1/retros/{date}`

One Retro as the `RetroView`. Errors: `404` the day has none.

### `GET /v1/lessons`

The lessons the Retro learned from earlier cards, newest first; the analyst writes them at the end of its answer from the
day's failed runs, review rejections and merged cards. Query: `repository` (an id; only that repository's lessons) and
`retired` (`true` to include retired lessons, which are left out otherwise). Response: an array of `Lesson`: `id`,
`repository_id`, `kind` (`failure`, `rejection` or `success`), `situation`, `lesson` (what went wrong or what worked),
`advice` (what to do), `cards` (the cards it was learned from), `retro_date` (null for none), `status` (`active` or
`retired`), `retired_reason` (null while active: `not retrieved for 60 days`, or `harmful: ...` when a trial run the
lesson was given to was negative), `created_at`, `last_retrieved_at` (null if never) and `retrievals`.

At the start of a `code` or `review` run, the up to 5 active lessons of the card's repository that best match the issue's
text are put in the prompt under "Lessons from earlier cards", and the Pi tool `lessons(query)` finds more (see the
[bridge protocol](bridge-protocol.md#post-moebiuslessons)). Each counts as retrieved.

### `GET /v1/lessons/{id}`

One `Lesson`. Errors: `404`.

### `PUT /v1/lessons/{id}`

Rewrites a lesson's text. Request: `{"situation", "lesson", "advice"}`, each non-empty and at most 1000 characters;
secrets are removed. Response: the `Lesson`. Errors: `404`; `422` for an empty or long part.

### `DELETE /v1/lessons/{id}`

Deletes a lesson, retired or not. Response: `{}`. Errors: `404`.

### `GET /v1/proposals`

All proposals, the newest first. Response: an array of `Proposal`, each with its trials.

### `POST /v1/proposals/{id}/trial`

Tries a proposal now, at any hour, as the schedule does inside `[trial_window]`: its status becomes `trying`, the trials
go on after the response, and each trial ends with the event `trial_done` and the proposal with `proposal_status`.
Request: no body. Response: the `Proposal`, with the status `trying`. Errors: `404` no such
proposal; `409` the proposal is not `open`, is not of the kind `harness`, or another proposal is being tried (only one is
at a time); `422` there is no harness configuration repository.

### `POST /v1/proposals/{id}/merge`

A person merges a `positive` or `untried` proposal: its branch is merged into the default branch of the harness
configuration repository's clone with a merge commit, pushed to `origin`, and the agents are read again. Then the decision
is recorded (kind `merge_proposal`, choice `merge_proposal`, decider `person`), the proposal's inbox item is resolved and
its status becomes `merged`. A merge that conflicts, and a push that fails, change nothing. Request: `{"reason": string}`
(optional). Response: the `Proposal`. Errors: `404` no such proposal; `409` the proposal is neither `positive` nor `untried`,
or its branch does not merge cleanly; `422` there is no harness configuration repository.

A `settings` proposal is merged by saving its `edits` to `moebius.toml` as `PUT /v1/settings` saves a change (refused
with `422`, and nothing written, when the result is not a valid configuration), which records `edit_settings` too; when
the file then has a key the running server only uses after a restart, the server restarts shortly after the response,
as after `POST /v1/server/restart`. A `repository` proposal is merged by committing its files on the branch
`moebius/proposal-<id>` of the repository's clone, from its default branch, and offering that branch as a change
through the repository's forge (a pull request on GitHub), with a `ready_to_merge` inbox item; the change then merges as
any change does, and `change` names it. Both record the decision as a `harness` proposal does, with the proposal's
`kind` and `diff` in its context.

### `POST /v1/proposals/{id}/dismiss`

A person dismisses a `positive` or `untried` proposal: the decision is recorded (kind `merge_proposal`, choice `dismiss`,
decider `person`), the inbox item is resolved and the status becomes `dismissed`. The repository is not changed, nor is
`moebius.toml` or any repository for a `settings` or `repository` proposal. Request and errors as for `merge`, without
the conflict. Response: the `Proposal`.

### `GET /v1/decision-model`

What the decision model has measured (see [the decision model](decision-model.md)). Response, a `DecisionModelView`:

```json
{"configured": true, "model": "moebius-decider", "agreement": 0.95, "min_judgements": 100,
 "kinds": [{"kind": "tool_call", "shadows": 120, "agreement": 0.97, "failed": 2, "dropped": 0, "routed": false}],
 "takeovers": [{"id": 1, "kind": "tool_call", "agreement": 0.97, "count": 120, "status": "open", "at": 1790000000}]}
```

`configured` is false without a `[decision_model]` table, and then `model`, `agreement` and `min_judgements` are null and
`kinds` is empty. Otherwise `model`, `agreement` (the share a takeover needs) and `min_judgements` are the configuration's.
`kinds` has an entry, kinds in alphabetical order, for each kind with a shadow, a dropped request or a merged takeover:
`shadows` (held-out judgements the model was asked about), `agreement` (the share of them where it chose the recorded
choice, null before there is a shadow), `failed` (the shadows where it failed, counted as disagreement), `dropped`
(requests dropped because the queue was full, since the server started) and `routed` (the model decides the kind now).
The kind `localize` is measured by the recall of its files and not by agreement (see
[`localize`](decision-model.md#localize)): its entry has a null `agreement` and besides `measured` (the cards whose
change is known), `recall` (the model's mean recall at ten over them) and `baseline_recall` (the recorded hints'), null
before there is one; it is listed whenever a decision model is configured. `takeovers` are all of them, oldest first.

### `GET /v1/takeovers`

All takeovers, oldest first. Response: an array of `Takeover`: `id`, `kind`, `agreement` (between 0 and 1, when it was
proposed), `count` (the shadows it was measured on), `status` (`open`, `merged`, `dismissed` or `released`) and `at`.
An `open` takeover has an open inbox item of the kind `takeover`.

### `POST /v1/takeovers/{id}/merge`

A person lets the decision model take the kind over. The decision is recorded (kind `takeover`, choice `merge`, decider
`person`), the inbox item is resolved, the status becomes `merged` and the kind is routed to the model from now on and
after a restart. Request: `{"reason": string}` (optional). Response: the `Takeover`. Errors: `404` no such takeover; `409`
the takeover is not `open`.

### `POST /v1/takeovers/{id}/dismiss`

A person refuses an `open` takeover: recorded as choice `dismiss`, the inbox item is resolved and the status becomes
`dismissed`. The kind is proposed again only after `min_judgements` more shadows. Request, response and errors as for
`merge`.

### `POST /v1/takeovers/{id}/release`

A person hands a `merged` kind back to the decider it had: recorded as choice `release`, the status becomes `released`.
Request and response as for `merge`. Errors: `404` no such takeover; `409` the takeover is not `merged`.

### `GET /v1/grillings`

All grillings, the newest first. Response: an array of `Grilling`.

### `GET /v1/grillings/{id}`

One `Grilling`. Errors: `404` no such grilling.

### `POST /v1/grillings/{id}/confirm`

A grilling is a chat with the built-in agent `Grilling-partner` in a repository (`POST /v1/chats`). The agent works in the
chat's worktree and ends by writing exactly one of `.moebius/grilling/issues.json` (a JSON array of `{"title": string,
"body": string, "blocked_by": [number]}`, where `blocked_by` are indexes into the array) or `.moebius/grilling/report.md`
(a research report). It never commits or pushes them. When the chat ends or is stopped, Moebius reads the files before
the worktree is removed and stores a `Grilling`:

| Result | `status` | Inbox item |
|---|---|---|
| `issues.json` is a list of issues, each with a title, whose `blocked_by` are in range, not the issue itself and not in a circle | `proposed` | `grilling_issues`, titled "N issues from a grilling" |
| no `issues.json`, and a `report.md` that is not blank | `reported`, with `report` | `grilling_report` |
| `issues.json` exists but is broken or does not hold together (it decides even when a report is there), or neither file exists | `empty`, with `reason` | none |

The person reads the proposal, may edit, drop and reorder the issues, and confirms with the final list. Request:
`{"issues": [{"title": string, "body": string, "blocked_by": [number]}]}`; `body` and `blocked_by` may be left out, and
`blocked_by` indexes this list, not the proposed one. The server

1. checks the list as it checked the agent's (at least one issue, every title not blank, no `blocked_by` out of range, the
   issue itself or in a circle) and that the status is `proposed`, and sets the status to `confirmed`;
2. makes one card per issue in the repository's `Backlog`, each blocked by the cards of its `blocked_by` (their numbers
   are the card's dependencies, as `card.blocked_by` reads them), blockers first. The cards are `confirmed`
   (`card.confirmed`), so `backlog.luau` moves them on to `Not started` without approval, where `ready.luau` holds each until
   its blockers are done. Without a forge the cards are Moebius cards, authored by `person`. On a GitHub repository each
   issue is opened first through the GitHub API, its body ending in a line `Blocked by #3, #4` for the numbers of the
   issues that block it (the line the poll reads), and its card gets the issue's number, so that the poll finds the card
   existing; its `author` is `person`;
3. records the decision (kind `confirm_grilling`, choice `confirm_grilling`, decider `person`, the context holding the
   grilling's id and the final list as JSON) and resolves the inbox item.

If opening an issue fails the request fails with the error of the forge, the status is `proposed` again and the issues
already opened stay on GitHub; the cards made so far stay. Response: the `Grilling`, `confirmed`. Errors: `404` no such
grilling; `422` the grilling is not `proposed`, or the list is not valid (the message says why).

### `POST /v1/grillings/{id}/dismiss`

A person dismisses the proposal of a `proposed` grilling, or reads the report of a `reported` one and dismisses it: the
inbox item is resolved and the status becomes `dismissed`. Nothing is made. Request: `{}`. Response: the `Grilling`. Errors:
`404` no such grilling; `422` it is not `proposed` or `reported`.

### `POST /v1/research`

Stores a brief for a research as a draft (see [the Autoresearcher](harness-configuration.md#autoresearch)). Nothing runs
until a person confirms it. Request: `{"repository": id, "goal": string, "measure": string, "better": "lower" | "higher",
"token_budget": number}`. `measure` is a shell command; it runs in a worktree of the repository with the environment
cleared (`PATH` and `HOME` only), and its result is the last number it prints on standard output; an exit status other
than 0, running longer than 10 minutes, or printing no number means the measurement failed. `better` says which way the
number should move. `token_budget` is the most tokens (input plus output) the research's runs may use, more than 0.
Response: the `ResearchView`. Errors: `404` no such repository; `422` a blank `goal` or `measure`, another `better`, or a
budget that is not more than 0.

`ResearchView`: `id`, `repository_id`, `goal`, `measure`, `better`, `token_budget`, `status`, `baseline` (the measurement
of the repository as it is, or null until it is measured), `reason` (why the research ended; empty before), `at`, `tokens`
(used by its runs so far) and `experiments` (array of `Experiment`, the first first: one for each lever).

`status` of a research: `draft` (stored, not confirmed), `confirmed` (waits for the trial window or for `start`),
`running`, `done` (a round proposed no more levers), `spent` (its runs used the budget), `stopped` (a person stopped it) or
`failed` (the baseline could not be measured, or the Autoresearcher's run failed; `reason` says which).

`Experiment`: `id`, `research_id`, `branch` (`research/<research id>-<name>`, the lever), `title` (the subject of the
lever's commit), `value` (what the measure printed on the branch; null when the measurement failed), `checks` (true or
false: the repository's own checks on the branch, or null when it has none or the measurement failed), `kept`, `reason`
(text), `change_id` (the change offered for a kept lever, or null) and `at`. A lever is kept when `value` is better than
`baseline` in the direction of `better` by more than 1 % of the baseline, and `checks` is not false.

### `GET /v1/research`

All research, the newest first. Response: an array of `ResearchView`.

### `GET /v1/research/{id}`

One research as the `ResearchView`. Errors: `404` no such research.

### `PUT /v1/research/{id}`

A person, or the Moebius agent, writes the brief of a draft anew before it is confirmed. Request: `{"goal": string,
"measure": string, "better": "lower" | "higher", "token_budget": number, "reason": string}`; `reason` is optional and is
kept with the decision; the brief is checked as for `POST /v1/research`. The decision is recorded (kind
`edit_research`, decider `person` or `agent:Moebius`; its context has `repository`, `goal`, `measure`, `better`,
`token_budget` and each of the four as before, `previous_goal` and so on) and the event `research_status` is sent with
the status `draft`. A brief that changes nothing records nothing. Response: the `ResearchView`. Errors: `404` no such
research; `409` it is not a draft; `422` as for `POST /v1/research`.

### `POST /v1/research/{id}/confirm`

A person confirms the brief of a draft: the decision is recorded (kind `confirm_brief`, choice `confirm`, decider
`person`) and the status becomes `confirmed`. The research then starts by itself inside `[trial_window]` (see
[configuration](configuration.md)), when no proposal is being tried and no research is running; the oldest confirmed one
goes first. Request: `{}`. Response: the `ResearchView`. Errors: `404` no such research; `409` it is not a draft.

### `POST /v1/research/{id}/start`

Starts a confirmed research now, at any hour. Its status becomes `running` and the research goes on after the response:
the baseline is measured in a fresh worktree of the repository at its default branch, then rounds follow. In a round the
Autoresearcher (a run with the purpose `research` and no card) proposes levers, each a branch
`research/<research id>-<name>` with one commit; an experiment measures each new branch and runs the repository's own
checks; each kept lever is offered as a change without a card, like the work of a chat: published through the
repository's forge, with a `ready_to_merge` inbox item (`data.change`, `data.branch`) and the event `change_offered`. A
person merges it with `POST /v1/changes/{id}/merge`. Before each round the tokens of the research's runs are compared with
the budget; the research is `spent` when they have reached it (a round may run over). It is `done` when a round leaves no
new lever branch. Every status change is the event `research_status` and every experiment the event `experiment_done`.
When the server starts, research that was `running` goes back to `confirmed`; started again it goes on with the baseline
and the experiments it has. Request: `{}`. Response: the `ResearchView`, `running`. Errors: `404` no such research; `409`
the research is not `confirmed`, or another research is running (only one is at a time).

### `POST /v1/research/{id}/stop`

A person stops a research that has not ended: the status becomes `stopped` and a research run that is going is stopped
like `POST /v1/runs/{id}/stop` does. Experiments done stay, and so do the lever branches in the repository's clone. Request:
`{}`. Response: the `ResearchView`. Errors: `404` no such research; `409` it has ended already.

## Events

`GET /v1/events` sends one JSON object per message: `{"kind": "<kind>", "data": ...}`. These are all the kinds.

| Kind | When | `data` |
|---|---|---|
| `repository_added` | A repository was added. | The `Repository`. |
| `repository_updated` | The logins a repository on a forge trusts were changed through the API. | The `Repository`. |
| `repository_removed` | A repository was removed through the API. | The `Repository`. |
| `file_changed` | A file of a repository was saved through the API. | `{"repository": id, "path": string}` |
| `script_changed` | A column's script was saved or removed for a repository, through the API. | `{"repository": id, "column": string, "path": string}` |
| `card_added` | An issue became a card: created through the API or found on GitHub, GitLab or Forgejo. | The `Card`. |
| `card_updated` | The title or body of a card changed: edited through the API, or changed on GitHub, GitLab or Forgejo. | The `Card`. |
| `card_moved` | A card moved to another column. | `{"card": id, "from": column, "to": column}` |
| `card_waiting` | The reason a card waits changed: set, replaced, or cleared because the card moved or its script did not wait. Not sent when it stays the same. | `{"card": id, "reason": string or null}` |
| `chat_renamed` | A chat was given a name, or its name was cleared, through the API. | `{"run": id, "title": string}` (the title the list of chats shows now) |
| `change_offered` | A card's change, the work of a chat or a kept lever of a research was offered to a person to merge. | The `Change` (`card_id` is null for a chat's or a lever's). |
| `change_updated` | A change was published or published again, its checks started or ended, or the checks or review of its pull request changed. | The `Change`. |
| `change_merged` | A change was merged, here or on its forge. | `{"change": id, "commit": sha or null}` |
| `change_closed` | A change's pull or merge request was closed on its forge without a merge. | `{"change": id}` |
| `attempt_judged` | An attempt of a card was judged by its tests and checks (`moebius.check_attempt`). | `{"card": id, "run": id, "result": "passed" \| "failed"}` |
| `attempts_settled` | A card's open attempts were settled (`moebius.settle_attempts`): one picked, or none passed. | `{"card": id, "picked": id or null, "runs": [id]}` |
| `run_started` | A harness process for a run started. | `{"run": id, "card": id or null}` (null for a chat) |
| `run_event` | Something happened in a run. | `{"run": id, "seq": number, "kind": string, "data": object}` (below) |
| `run_ended` | A run ended. | `{"run": id, "status": "succeeded" \| "failed" \| "stopped"}` |
| `board_error` | A column script failed; the card stays where it is. | `{"card": id, "message": string}` |
| `forge_error` | Polling a repository on GitHub, GitLab or Forgejo, or refreshing the clone of a repository without a forge, failed; it is tried again at the next poll or refresh (every 30 seconds without a forge). | `{"repository": "owner/repo" (GitHub, Forgejo), the project's path (GitLab) or the repository's name, "message": string}` |
| `harness_error` | An agent's files in the harness configuration repository are broken (it is left out), or refreshing the repository failed (every 30 seconds). | `{"agent": name (left out when the refresh failed), "message": string}` |
| `package_installed` | A package was installed or updated (a plugin is in place and started; the files of the others are proposed). | `{"name": string, "version": string, "kind": "plugin" \| "workflow" \| "rules" \| "skills"}` |
| `package_removed` | A package was removed. | `{"name": string, "kind": string}` |
| `package_update_available` | The nightly check found a newer version that works with this Moebius; it is in the inbox. | `{"name": string, "version": string}` |
| `package_check_error` | The nightly check could not read a registry, or failed. | `{"registry": string (empty when the check itself failed), "message": string}` |
| `retro_done` | A Retro ended: it was `reported`, or the analyst's run ended and its proposals were stored (`proposed`), or it `failed`. | `{"date": "YYYY-MM-DD", "status": "reported" \| "proposed" \| "failed", "proposals": number}` |
| `retro_error` | The schedule could not start the day's Retro, the trials of a proposal, a research or the replays, or the trials of a proposal broke off; it is tried again a minute later. | `{"message": string}` |
| `replayed` | A refused tool call was replayed and its judgement has the outcome. | `{"judgement": id, "run": id, "outcome": string}` |
| `trial_done` | One trial of a proposal ended and was stored. | `{"proposal": id, "card": id or null, "outcome": "positive" \| "negative" \| "skipped" \| "failed", "regression": bool}` |
| `proposal_status` | A proposal's status changed: it is being tried, was tried, was merged or dismissed, or a proposal for Moebius's own configuration was stored, `untried`. | `{"proposal": id, "status": string}` (the statuses above) |
| `takeover_proposed` | The decision model earned a takeover proposal for a kind, which is in the inbox. | `{"takeover": id, "kind": string, "agreement": number}` |
| `takeover_status` | A takeover was merged, dismissed or released. | `{"takeover": id, "status": string}` (the statuses above) |
| `research_status` | A research was stored, its brief was edited, or it was confirmed, started, stopped or ended. | `{"research": id, "status": string}` (the statuses above) |
| `experiment_done` | An experiment of a research ended and was stored. | `{"research": id, "lever": the lever's branch, "value": number or null, "kept": boolean}` |
| `automation_fired` | An automation fired, by its schedule or `POST /v1/automations/{id}/run`. | `{"automation": id, "firing": id, "status": "started" \| "skipped" \| "failed", "run": id or null}` |
| `grilling_ended` | A grilling chat ended and its result was stored. | `{"grilling": id, "status": "proposed" \| "reported" \| "empty"}` |
| `port_opened` | The server noticed a port that a run, chat or terminal listens on (looked for every 3 seconds while a run or terminal exists). | `{"port": integer, "pid": integer, "process": string, "kind": "run" \| "chat" \| "terminal", "run": id or null}` (as `GET /v1/ports`) |
| `port_closed` | A port that was listed is no longer listened on, or its run or terminal is gone. | The same as `port_opened`, for the port as it was. |
| `lagged` | This client fell too far behind (more than 1024 events) and lost the oldest. | `{"missed": number}` |

A `run_event`'s source is told by its inner `kind`:

- `tool_check`: the answer to a tool-call check, `{"tool", "tool_call_id", "decision": "allow" | "refuse", "reason"}`. See
  the [bridge protocol](bridge-protocol.md).
- `mcp_call`: a call of a tool of an MCP server through the MCP gateway, `{"server", "tool", "tool_call_id", "input",
  "decision", "reason", "result"}`, where `result` is what the harness was given (the refusal for a refused call). See
  the [MCP gateway](bridge-protocol.md#the-mcp-gateway).
- `tool_output`: a tool's result made smaller before the model read it, `{"tool", "tool_call_id", "steps", "bytes_before",
  "bytes_after", "lines_before", "lines_after", "saved"}`. See the [bridge protocol](bridge-protocol.md#post-moebiusresult).
- `lens_check`: one check of the lens after an edit or before the run settled, `{"file", "tool_call_id", "phase": "syntax" |
  "lint" | "deep", "tool", "ms", "skipped", "findings", "shown"}`: how long the check took, why it did not run when it
  did not (`over budget`, `not installed`, or why `.moebius/lint.toml` cannot be read), how many problems it found and how
  many the agent was told about; `file` and `tool_call_id` are null for `deep`. See [the lens](bridge-protocol.md#the-lens).
- `mcp_error`: an MCP server of the run's agent could not be started or reached, `{"server", "message"}`; its tools are
  left out.
- `launcher_note`: what the launcher plugin said about starting the run's process, `{"note"}` (see
  [launchers](plugin-api.md#launchers)), such as that the run is on a runner and is not wrapped.
- Every other kind is a record Pi-Bolt wrote on its RPC channel, with its `type` as the kind (for example
  `agent_settled`, `tool_execution_end`, `response`) and the whole record as `data`. A line that is not JSON is passed on
  as kind `unparsed` with `{"type": "unparsed", "error", "text"}`. A run on Claude Code (`harness` is `claude-code`)
  has the same kinds, made of Claude Code's `stream-json` records: `agent_start` when it is given a message, an
  `assistant` record as `message_start`, a `message_update` (`text_delta` or `thinking_delta`) per text and thinking
  block, a `tool_execution_start` per `tool_use` block and `message_end`; each `tool_result` as a `tool_execution_end`
  (`{"toolCallId", "toolName", "result": {"content"}, "isError"}`); a `result` as `agent_end` (its text the final answer,
  with `subtype`, `is_error`, `num_turns`, `duration_ms`) and `agent_settled` (`{"aborted": false}`). Any other record is
  kept whole as `claude_<type>` (such as `claude_system`), and a line that is not JSON as `claude_unparsed` with
  `{"text"}`.

A provider that refuses the key, has no quota left or answers an HTTP error does not end the run: Pi-Bolt ends its assistant
message with `"stopReason": "error"` and an `"errorMessage"`, in the `message` of a `message_end` record and in the
`messages` of the `agent_end` that follows (a terminal's record has only the `agent_end`); a Claude Code run ends with an
`agent_end` that has `"is_error": true`. A client shows that as the model's error in the conversation. A run that
Moebius ends in failure has a `moebius_error` event instead.

`seq` is the number the event is stored under, the same as in `GET /v1/runs/{id}/events`. Events of a run are published in
the order of their `seq`. Every event of a run is published and stored: Pi's records, `tool_check`, `mcp_call`, `mcp_error` and `launcher_note` events, `steered` events
(`{"message"}`, a person's steer, with secrets removed), `said` events (`{"message"}`, a person's message to a chat, the first
one included, with secrets removed; stored before the agent's records of its answer) `shell_opened` events (`{"who"}`, a person opened a shell in the worktree; see `GET /v1/runs/{id}/terminal`), `tui_opened` events (`{"who"}`, a person opened Pi-Bolt's terminal interface in a chat; see `GET /v1/chats/{id}/tui`), `settings_changed` events (`{"model", "protocol", "thinking"}`, what a chat runs with in place of its agent's; see `POST /v1/chats/{id}/settings`) and `moebius_error` events (`{"message"}`) that explain why a
run failed. The one exception is a run that was still going when the server stopped: it is failed at the next start with
a `moebius_error` that is stored but not published. A chat that had answered and was only waiting for the next message is
the exception to that: nothing was going in it, so it is `stopped` at the next start and nothing is added to it.
