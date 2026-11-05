# Keeping free pools current (model discovery)

Free providers add and remove models all the time. A pool you wrote once, or migrated once from
ModelRelay, slowly goes stale: new free models never join, and removed ones linger. Discovery keeps a pool
current **without you tending it**, in a way that stays safe: it only ever proposes **free chat models**, only
for providers you switch on, and only changes a **generated file** that is separate from your own
configuration. **The gateway does not run discovery for you: you schedule one command** (recipes below).
ModelRelay refreshed itself every 30 minutes; with humpyard you choose the schedule.

## What it does

- `humpyard models check` reports what discovery would change. It never writes anything. It exits 0 when there
  is nothing to review, 1 when there is, and 2 when a source or the config could not be used, so a scheduled job
  can alert on it.
- `humpyard models sync` rebuilds the generated file (`[discovery] file`), only when its content changes, and
  keeps the previous version next to it as `.prev`. `--every 6h` repeats it in a loop.
- `serve` only **reads** that file, at load and on every reload, and never fetches anything. A file that is
  missing, empty or invalid is ignored and the gateway serves with your own configuration.

A model is **added** only when two independent sources agree: the open [models.dev](https://models.dev)
dataset says it is a free chat model with a context window, and the provider's own `/models` lists it for your
key. A model already in the file is **kept** while the provider still lists it and the dataset has not recorded
a cost for it (a dataset that lags or forgets a model never removes one), and **removed** when the provider
stops listing it or it turns paid. Models are named `auto-<provider>-<model>` and get the strengths you set for
the provider plus `all` (and `reasoning` when the dataset says so).

## Switching it on, and off

Discovery is **off until you opt in, at two levels**:

```toml
[discovery]
enabled = true                       # the master switch (default false); needs `file`
file = "/var/lib/humpyard/discovered.toml"   # an absolute path; sync writes it, serve reads it
# class = "public"                   # applied to every generated target, whatever the file says
# max_added_per_run = 10             # a run that would add more is refused
# max_removed_fraction = 0.5         # a run that would remove more of the generated set is refused
# exclude = ["groq/whisper-large-v3"]

[providers.groq]
base_url = "https://api.groq.com/openai/v1"
api_key_env = "GROQ_API_KEY"
price_source = "models-dev:groq"     # which models.dev provider to read
discover = { strengths = ["general"] }   # opts this provider in (default enabled = true, confirm = true)
```

- **Per provider:** a provider without `discover` is never touched. `discover = { ..., enabled = false }` opts it
  out again without deleting its settings: its generated targets stop being loaded at once and the next sync
  removes them. `confirm = false` skips the provider's own `/models` check (weaker; only for a provider with no
  usable `/models`).
- **Globally:** `[discovery] enabled = false` (or leaving it out) stops everything: `serve` does not read the
  file and `models sync` refuses to write. `models check` still works, because it only reports.

Prices, keys, routes and anything you wrote by hand are never changed. Generated targets only ever carry the
class from `[discovery] class` (so they are removed on strict private routes), are priced at zero, and cannot
duplicate a model you wrote yourself.

## What it cannot know

Providers do not say what a model is good at, so new models get only the strengths you set for the provider
(plus `all`, and `reasoning` when the dataset says so). They join `tag:general`-style pools, not `coding` ones,
until you write a target of your own for that model. "Free" means $0 per token in the dataset, not unlimited:
free tiers have rate limits, which `Retry-After` handling and health skipping absorb.

For a provider on a **paid plan** (for example OpenCode Go), nothing is ever added automatically, because adding
a paid model costs money. `humpyard models check --paid` lists the new paid chat models with their price and
window so you can add the ones you want by hand (`humpyard prices import` proposes the price).

## Finding models a provider has retired

Discovery and `prices check` compare your configuration with the models.dev dataset, which can lag behind a
provider by days or weeks. When a provider retires a model your targets still use, requests to it fail over and
nobody is told. `humpyard models check --retired` asks each provider for its **own** model list instead and reports
every configured endpoint, hand-written or generated, whose model the provider no longer lists:

```
$ humpyard models check --retired --config /etc/humpyard/config.toml
retired:    pf3 uses opencode/mimo-v2.5, which the provider no longer lists
```

It needs no `discover` and no `price_source`, sends each provider's key only to that provider's own host (redirects
are not followed, and the response is size- and time-limited), never writes a file and never changes the
configuration. `--provider NAME` checks one provider and `--json` prints the report.

A provider whose list is missing is one of three things, and the check says which:

- **No list** (`/models` answers 404, 405 or 501): the provider has nothing to compare with. Reported as `no list`.
- **Unreachable** (the connection fails or times out, or the answer is 502, 503 or 504): the check could not ask,
  for example a laptop that is asleep. Reported as `unreachable`.
- **Unusable** (a rejected key, any other error status, a body that is not a model list, an empty list): the
  provider answered but the check cannot use it. Reported as `unusable`.

It exits **0** when no endpoint is reported, **1** when an endpoint is, and **2** when a provider is unusable. A
provider with no list or one that is unreachable is in the report but does not change the exit code, so a scheduled
job that alerts on a non-zero exit stays quiet until something needs a person; whether a provider is up is the job
of the health metrics and `humpyard_endpoint_unavailable_total`. In every case the other providers are still
checked. A provider with no key is skipped and listed as `skipped`. The JSON report has `no_list`, `unreachable`,
`unusable` and `skipped` beside `checked` and `retired`.

A report is evidence, not proof. A provider can serve a model it does not list (an alias, a preview or hidden
model), and a local provider lists only what is pulled, so look before you remove a target. To be told sooner, watch
`humpyard_endpoint_unavailable_total` ([metrics](metrics.md)), which counts the attempts a provider answers with
"this model is gone", "your key is rejected" or "no credit".

A weekly check beside the price check is plenty, for example the cron entry
`23 6 * * 1  humpyard  . /etc/humpyard/env && /usr/local/bin/humpyard models check --retired --config /etc/humpyard/config.toml`.

## Scheduling it

Run `models sync` on a schedule, on the machine or container that has the generated file where the gateway can
read it. The gateway's own reload poll (`reload_poll_secs`, default every few seconds) picks up a changed file;
nothing else needs to be told. Provider keys come from the same environment variables the gateway uses, so the
scheduled job needs them too. Every 6 hours is plenty: lineups change over days, not minutes.

### cron (any Unix machine)

```
# /etc/cron.d/humpyard-discovery
17 */6 * * *  humpyard  . /etc/humpyard/env && /usr/local/bin/humpyard models sync --config /etc/humpyard/config.toml >> /var/log/humpyard-discovery.log 2>&1
```

`/etc/humpyard/env` holds the `*_API_KEY` variables (mode 0600). Cron's own environment is nearly empty, so the
job sources it.

### systemd timer

```ini
# /etc/systemd/system/humpyard-discovery.service
[Unit]
Description=humpyard model discovery

[Service]
Type=oneshot
User=humpyard
EnvironmentFile=/etc/humpyard/env
ExecStart=/usr/local/bin/humpyard models sync --config /etc/humpyard/config.toml
```

```ini
# /etc/systemd/system/humpyard-discovery.timer
[Unit]
Description=humpyard model discovery, every 6 hours

[Timer]
OnCalendar=*-*-* 00/6:17:00
RandomizedDelaySec=300
Persistent=true

[Install]
WantedBy=timers.target
```

`systemctl enable --now humpyard-discovery.timer`; read the result with `journalctl -u humpyard-discovery`.

### launchd (macOS)

Put the keys in a small wrapper script (mode 0700) so they are not in the plist:

```sh
#!/bin/sh
# /usr/local/bin/humpyard-discover.sh
. /etc/humpyard/env
exec /usr/local/bin/humpyard models sync --config /etc/humpyard/config.toml
```

```xml
<!-- ~/Library/LaunchAgents/dev.humpyard.discovery.plist -->
<plist version="1.0"><dict>
  <key>Label</key><string>dev.humpyard.discovery</string>
  <key>ProgramArguments</key><array><string>/usr/local/bin/humpyard-discover.sh</string></array>
  <key>StartInterval</key><integer>21600</integer>
  <key>RunAtLoad</key><true/>
  <key>StandardOutPath</key><string>/tmp/humpyard-discovery.log</string>
  <key>StandardErrorPath</key><string>/tmp/humpyard-discovery.log</string>
</dict></plist>
```

`launchctl load ~/Library/LaunchAgents/dev.humpyard.discovery.plist`.

### Windows Task Scheduler

Set the `*_API_KEY` variables for the account the task runs as, then:

```
schtasks /Create /SC HOURLY /MO 6 /TN humpyard-discovery /TR "C:\humpyard\humpyard.exe models sync --config C:\humpyard\config.toml"
```

### Docker, with a scheduler on the host

Keep the generated file on a directory both the gateway and the job mount, and run the one-shot job from the
host's cron or timer:

```
17 */6 * * *  root  docker run --rm --env-file /etc/humpyard/env -v /etc/humpyard:/etc/humpyard gschaetz/humpyard:<version> models sync --config /etc/humpyard/config.toml
```

### No scheduler available: the loop

`humpyard models sync --every 6h` stays running and repeats; a failed run is logged and the loop goes on. Run it
as a second container or service next to the gateway (for example another service in a compose file, with the
same volume for the generated file):

```yaml
  discovery:
    image: gschaetz/humpyard:<version>
    command: ["models", "sync", "--config", "/etc/humpyard/config.toml", "--every", "6h"]
    env_file: /etc/humpyard/env
    volumes: ["/etc/humpyard:/etc/humpyard"]
```

### Kubernetes

The simplest form is a **sidecar** in the gateway's pod running the loop, with an `emptyDir` that both
containers mount where `[discovery] file` points (the file is rebuilt after a restart). The image has no shell,
so pass arguments only:

```yaml
- name: discovery
  image: gschaetz/humpyard:<version>
  args: ["models", "sync", "--config", "/etc/humpyard/config.toml", "--every", "6h"]
  envFrom: [{ secretRef: { name: humpyard-provider-keys } }]
  volumeMounts:
    - { name: config, mountPath: /etc/humpyard, readOnly: true }
    - { name: discovered, mountPath: /var/lib/humpyard }
```

A **CronJob** also works, but it needs storage that the gateway pod can read and the job can write (a
ReadWriteMany volume), which is why the sidecar is simpler. The pod needs outbound HTTPS to models.dev and to
each provider.

## Watching it

`humpyard check-config` says what became of the file. The gateway exposes `humpyard_discovery_file_state`
(`disabled`, `missing`, `empty`, `loaded`, `ignored`) and `humpyard_discovery_targets`, `route/explain` marks
generated targets with `generated = true`, and the sync output lists every addition and removal, so a Slack alert
on a failed scheduled job (or on `ignored`) is enough.

See [ADR 0022](adr/0022-free-pool-membership-may-be-updated-on-a-schedule-without-review.md).
