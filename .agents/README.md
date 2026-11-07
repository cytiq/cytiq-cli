# Agent skills

Keep shared skills in `.agents/skills/<name>/`, with `SKILL.md` and any
supporting files together. This directory is the single source for both
Codex and Claude Code.

`.claude/skills` is a relative symbolic link to `../.agents/skills`.
Add or update skills here; both agents see the same files without extra
copies or per-skill links. Keep Claude-specific state in `.claude/`.
