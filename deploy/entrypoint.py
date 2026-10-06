#!/usr/bin/env python3
"""Container host adapter: env -> original JS config and fixed TCP destinations."""
import json
import os
from pathlib import Path
import socket
import sys
from urllib.parse import urlsplit


def endpoint(host, port):
    port = int(port)
    if not 1 <= port <= 65535:
        raise ValueError("TCP port must be 1..65535")
    host = host.removeprefix("[").removesuffix("]")
    addresses = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    # Prefer IPv4 on Docker bridges, retaining IPv6-only destinations.
    addresses.sort(key=lambda a: a[0] != socket.AF_INET)
    address = addresses[0][4][0]
    return f"[{address}]:{port}" if ":" in address else f"{address}:{port}"


def override_endpoint(value, host, port):
    if not value:
        return endpoint(host, port)
    if value.startswith("["):
        end = value.index("]")
        if value[end + 1:end + 2] != ":":
            raise ValueError("IPv6 upstream must be [address]:port")
        return endpoint(value[1:end], value[end + 2:])
    name, number = value.rsplit(":", 1)
    return endpoint(name, number)


def main():
    client = Path("/wow")
    explicit_data = os.getenv("WOW_DATA_DIR")
    data = Path(explicit_data) if explicit_data else next(
        (p for p in client.iterdir() if p.is_dir() and p.name.lower() == "data"),
        client / "Data",
    )
    if not data.is_dir():
        raise ValueError(f"WoW Data directory absent: {data}; mount the client at /wow read-only")

    shared_host = os.getenv("BENILLA_HOST", "").strip()
    public_url = os.getenv("BENILLA_PUBLIC_URL", "").strip()
    if not public_url and shared_host:
        host = shared_host if ":" not in shared_host or shared_host.startswith("[") else f"[{shared_host}]"
        port = int(os.getenv("BENILLA_PUBLIC_PORT", "8000"))
        if not 1 <= port <= 65535:
            raise ValueError("BENILLA_PUBLIC_PORT must be 1..65535")
        public_url = f"http://{host}:{port}"
    if public_url:
        parsed = urlsplit(public_url)
        if (parsed.scheme not in ("http", "https") or not parsed.hostname
                or parsed.username is not None or parsed.password is not None
                or parsed.query or parsed.fragment):
            raise ValueError("BENILLA_PUBLIC_URL must be an HTTP(S) URL without credentials/query/fragment")
        parsed.port  # Validate port syntax too.

    upstream = os.getenv("UPSTREAM_HOST") or shared_host or "host.docker.internal"
    auth = override_endpoint(os.getenv("AUTH_UPSTREAM"), upstream, os.getenv("AUTH_PORT", "3724"))
    world = override_endpoint(os.getenv("WORLD_UPSTREAM"), upstream, os.getenv("WORLD_PORT", "8085"))
    root = Path("/run/benilla/web")
    root.mkdir(parents=True, exist_ok=True)
    source = Path("/opt/benilla/web")
    for path in source.iterdir():
        if path.name != "config.js":
            link = root / path.name
            if not link.is_symlink():
                link.symlink_to(path, target_is_directory=path.is_dir())
    (root / "config.js").write_text(
        (source / "config.js").read_text() + "\n// Runtime environment override.\n"
        + "globalThis.BENILLA_CONFIG.gateway = " + json.dumps(public_url) + ";\n"
    )
    print(f"Benilla: public gateway={public_url or '(page origin)'}, auth={auth}, world={world}, Data={data}", flush=True)
    os.execv("/usr/local/bin/benilla-asset-gateway", [
        "benilla-asset-gateway", "--data", str(data), "--web", str(root),
        "--bind", "0.0.0.0:8000", "--auth-upstream", auth, "--world-upstream", world,
        *sys.argv[1:],
    ])


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, IndexError) as error:
        print(f"Benilla startup failed: {error}", file=sys.stderr)
        sys.exit(1)
