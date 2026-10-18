FROM rust:1.95-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl gnupg apt-transport-https git python3 python3-venv strace \
      iproute2 util-linux procps \
 && curl -fsSL https://deb.torproject.org/torproject.org/A3C4F0F979CAA22CDBA8F512EE8CBC9E886DDD89.asc \
      | gpg --dearmor -o /usr/share/keyrings/deb.torproject.org-keyring.gpg \
 && echo "deb [signed-by=/usr/share/keyrings/deb.torproject.org-keyring.gpg] https://deb.torproject.org/torproject.org bookworm main" \
      > /etc/apt/sources.list.d/tor.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends tor \
 && rm -rf /var/lib/apt/lists/*

RUN rustup toolchain install 1.95.0 --profile minimal --component rustfmt,clippy \
 && rustup toolchain install 1.85.1 --profile minimal \
 && rustup toolchain install stable --profile minimal

RUN git clone https://gitlab.torproject.org/tpo/core/chutney.git /opt/chutney \
 && git -C /opt/chutney checkout ae3a33c \
 && python3 -m venv /opt/chutney-venv \
 && /opt/chutney-venv/bin/pip install --no-cache-dir /opt/chutney \
 && git -C /opt/chutney rev-parse HEAD > /opt/chutney.commit
ENV PATH=/opt/chutney-venv/bin:$PATH

RUN useradd -m -u 1000 verifier
USER verifier
WORKDIR /home/verifier
