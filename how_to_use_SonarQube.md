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

The token can be provided in three ways (in order of precedence):
1. Pasted directly into the **Token** field in Settings
2. Set as `SONAR_TOKEN` in your shell environment
3. Set as `SONAR_TOKEN` in a `.env` file in your `.Dirigent/` directory

## Required Permissions

The token's user must have the following permissions on the SonarQube project (configured in **SonarQube Project Settings > Permissions**):

| Data                    | Required Permission                           |
|-------------------------|-----------------------------------------------|
| Issues                  | **Browse**                                    |
| Security Hotspots       | **Browse** + **Administer Security Hotspots** |
| Duplication metrics     | **Browse**                                    |
| Per-file duplications   | **Browse**                                    |

If permissions are missing, Dirigent will skip the affected data gracefully and print a warning to the terminal, e.g.:

```
SonarQube hotspots skipped: source: SonarQube API error: Insufficient privileges
  → Grant 'Browse' + 'Administer Security Hotspots' permission to your token's user
    in SonarQube Project Settings > Permissions
```

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
| `Insufficient privileges` | Token user lacks permissions | Grant permissions per table above |
| `SonarQube request failed` | Server unreachable or URL wrong | Check the Host URL and network |
