# How to use SonarQube with Dirigent

Dirigent can pull issues, security hotspots, and code duplication data from a SonarQube instance and display them as cues.

## Setup

1. Open **Settings > Sources** in Dirigent
2. Add a new source and select **SonarQube**
3. Fill in the fields:
   - **Host URL** — your SonarQube server (e.g. `http://localhost:9000`)
   - **Project Key** — the SonarQube project key (e.g. `my-project`)
   - **Token** — a SonarQube user token (see below)

### Token

Generate a token in SonarQube under **My Account > Security > Generate Tokens**.

**You must create a User Token (prefix `squ_`).** SonarQube offers three token types:

| Token type | Prefix | Works for Dirigent? |
|---|---|---|
| **User Token** | `squ_` | **Yes** — full API read access (issues, hotspots, duplications) |
| Project Analysis Token | `sqp_` | **No** — can only upload scan results, cannot read any API data |
| Global Analysis Token | `sqg_` | **No** — same as above, just not scoped to one project |

When generating the token, select **User Token** from the Type dropdown. If you accidentally created a `sqp_` or `sqg_` token, it will fail with "Insufficient privileges" on every API call regardless of what project permissions are set.

The token can be provided in three ways (in order of precedence):
1. Pasted directly into the **Token** field in Settings
2. Set as `SONAR_TOKEN` in your shell environment
3. Set as `SONAR_TOKEN` in a `.env` file in your `.Dirigent/` directory

## Required Permissions

Once you have a **User Token** (`squ_`), the token's user must also have the following project-level permissions (configured in SonarQube at **Project Settings > Permissions**, or via the URL `/project_roles?id=YourProjectKey`):

| Data                    | Required Permission                           |
|-------------------------|-----------------------------------------------|
| Issues                  | **Browse**                                    |
| Security Hotspots       | **Browse** + **Administer Security Hotspots** |
| Duplication metrics     | **Browse**                                    |
| Per-file duplications   | **Browse**                                    |

If permissions are missing, Dirigent will skip the affected data gracefully and print a warning to the terminal.

## What Dirigent fetches

Dirigent makes three categories of API calls:

1. **Issues** (`/api/issues/search`) — open bugs, vulnerabilities, and code smells (up to 100)
2. **Security Hotspots** (`/api/hotspots/search`) — items marked TO_REVIEW (up to 100)
3. **Duplications** (`/api/measures/component` + `/api/measures/component_tree`) — duplication density, duplicated blocks/lines/files

Each finding becomes a cue in the Dirigent Inbox with severity, rule key, and file location.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `SonarQube token is empty` | No token configured | Add token in Settings, env, or `.env` |
| `SonarQube host URL is empty` | Host URL blank | Set the Host URL field |
| `SonarQube project key is empty` | Project key blank | Set the Project Key field |
| `Insufficient privileges` | **Most likely:** token is a `sqp_` (analysis-only) token, not a `squ_` (user) token. **Less likely:** user lacks project permissions. | Check your token prefix. If it starts with `sqp_`, generate a new **User Token** (`squ_`). If already `squ_`, grant permissions per table above. |
| `SonarQube request failed` | Server unreachable or URL wrong | Check the Host URL and network |
