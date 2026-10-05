# The environment of the private Tor network test: Debian 12, Tor from the
# Tor Project's repository, Chutney, strace and Python. Monolith is built
# on the host and its target directory mounted at /target; the repository
# at /src. Build and run:
#
#     docker build -t monolith-tor-network tests/tor-network
#     docker run --rm --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
#         -v "$PWD:/src:ro" -v "$PWD/target:/target:ro" \
#         monolith-tor-network /src/tests/tor-network/four-node.sh
#
# Test tooling installs and starts Tor here; Monolith never does.
FROM debian:12

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        apt-transport-https ca-certificates gnupg wget git python3 python3-venv strace procps \
 && wget -qO- https://deb.torproject.org/torproject.org/A3C4F0F979CAA22CDBA8F512EE8CBC9E886DDD89.asc \
        | gpg --dearmor > /usr/share/keyrings/deb.torproject.org-keyring.gpg \
 && echo "deb [signed-by=/usr/share/keyrings/deb.torproject.org-keyring.gpg] https://deb.torproject.org/torproject.org bookworm main" \
        > /etc/apt/sources.list.d/tor.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends tor \
 && rm -rf /var/lib/apt/lists/*

# Chutney at the commit the Phase 3 test ran with.
RUN git clone https://gitlab.torproject.org/tpo/core/chutney.git /opt/chutney \
 && git -C /opt/chutney checkout ae3a33c \
 && python3 -m venv /opt/chutney-venv \
 && /opt/chutney-venv/bin/pip install --no-cache-dir /opt/chutney

ENV CHUTNEY_PATH=/opt/chutney
ENV PATH=/opt/chutney-venv/bin:$PATH
