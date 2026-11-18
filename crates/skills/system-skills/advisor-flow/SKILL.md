---
name: advisor-flow
description: Use when the user requests an Advisor second opinion or asks how to configure or activate Advisor.
---

# Advisor consultations

The user selects the Advisor provider and model in the shared settings catalog. In the terminal, `/advisor-settings` opens those settings, `/advisor` activates the workflow and `/advisor-off` deactivates it. A user can also run `/advisor-manual [question]` for one consultation while the workflow is inactive.

When `ask_advisor` is available, ask the decision you need help with in `question`. Put your proposed approach and the checks you have actually run in the optional `draft`. Both fields go to the configured Advisor provider. Keep any evidence the user withheld out of these fields.

Evaluate the answer against the workspace and the user's requirements. The Executor remains responsible for edits and verification. Identify any assumption in the advice that would change the outcome before acting on it.
