FROM node:22-trixie-slim

# The runsc image must stay on glibc 2.39+ so mounted host-built
# auto-runs/auto-runs-worker binaries can start inside the container.

ENV DEBIAN_FRONTEND=noninteractive \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    NPM_CONFIG_FUND=false

ARG CLAUDE_CODE_VERSION=2.1.94
ARG CODEX_VERSION=0.116.0
ARG GEMINI_CLI_VERSION=0.37.0
ARG QWEN_CODE_VERSION=0.14.0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        coreutils \
        curl \
        findutils \
        git \
        jq \
        procps \
        python3 \
        ripgrep \
        tini \
    && npm install -g \
        @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION} \
        @google/gemini-cli@${GEMINI_CLI_VERSION} \
        @openai/codex@${CODEX_VERSION} \
        @qwen-code/qwen-code@${QWEN_CODE_VERSION} \
    && rm -rf /var/lib/apt/lists/*

COPY scripts/validate_autonomous_task_report.py /usr/local/bin/validate_autonomous_task_report.py
RUN chmod +x /usr/local/bin/validate_autonomous_task_report.py

ENTRYPOINT ["/usr/bin/tini", "--"]
