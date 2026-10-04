"""HTTPS client for the pinned SFTS GitHub repo only."""

from __future__ import annotations

import ipaddress
import re
import socket
from urllib.parse import urlparse

import httpx

PIN_OWNER = "mimateinn"
PIN_REPO = "Versora"
GITHUB_HOSTS = frozenset({"api.github.com", "codeload.github.com"})

_TAG_RE = re.compile(r"^v?\d+(?:\.\d+)*[A-Za-z0-9._-]*$")
_SHA_RE = re.compile(r"^[0-9a-f]{7,40}$")


class GitHubHostError(ValueError):
    """Raised when a GitHub URL is not the pinned official overlay path."""


def is_release_tag(tag: str) -> bool:
    t = (tag or "").strip()
    if not t or t in {"main", "master", "HEAD"}:
        return False
    if t.startswith("refs/heads/") or "/heads/" in t:
        return False
    return bool(_TAG_RE.match(t))


def _host_public(host: str) -> None:
    try:
        infos = socket.getaddrinfo(host, None)
    except socket.gaierror as e:
        raise GitHubHostError("Could not resolve host.") from e
    ips = {item[4][0] for item in infos if item[4]}
    if not ips:
        raise GitHubHostError("Could not resolve host.")
    for ip in ips:
        addr = ipaddress.ip_address(ip)
        if addr.is_private or addr.is_loopback or addr.is_link_local:
            raise GitHubHostError("That address is not allowed.")
        if addr.is_reserved or addr.is_multicast or addr.is_unspecified:
            raise GitHubHostError("That address is not allowed.")
        if str(addr) == "169.254.169.254":
            raise GitHubHostError("That address is not allowed.")


def assert_github_overlay_url(url: str, expected_tag: str | None = None) -> str:
    raw = (url or "").strip()
    parsed = urlparse(raw)
    if parsed.scheme.lower() != "https":
        raise GitHubHostError("Only HTTPS is allowed.")
    host = (parsed.hostname or "").lower().rstrip(".")
    if host not in GITHUB_HOSTS:
        raise GitHubHostError("Host is not on the GitHub overlay allowlist.")
    try:
        ipaddress.ip_address(host)
    except ValueError:
        pass
    else:
        raise GitHubHostError("Raw IP addresses are not allowed.")
    _host_public(host)
    path = parsed.path or ""
    owner = PIN_OWNER
    repo = PIN_REPO
    if host == "api.github.com":
        prefix = f"/repos/{owner}/{repo}/"
        if not path.startswith(prefix):
            raise GitHubHostError("Path is not the pinned repository.")
        rest = path[len(prefix) :]
        if rest.startswith("releases"):
            return raw
        if rest.startswith("git/ref/tags/") or rest.startswith("git/refs/tags/"):
            tag = rest.split("tags/", 1)[-1]
            if not is_release_tag(tag):
                raise GitHubHostError("Not an official release tag.")
            return raw
        if rest.startswith("commits/"):
            ref = rest.split("commits/", 1)[-1]
            if not (is_release_tag(ref) or _SHA_RE.match(ref)):
                raise GitHubHostError("Not an official tag or commit.")
            return raw
        if rest.startswith("zipball/"):
            tag = rest.split("zipball/", 1)[-1]
            tag = tag.removeprefix("refs/tags/")
            if not is_release_tag(tag):
                raise GitHubHostError("Zipball must be an official release tag.")
            if expected_tag and tag != expected_tag.removeprefix("refs/tags/"):
                raise GitHubHostError("Zipball tag does not match the chosen release.")
            return raw
        raise GitHubHostError("API path is not allowed.")
    # codeload.github.com — official tag zip only
    parts = [p for p in path.split("/") if p]
    # /owner/repo/legacy.zip/refs/tags/TAG
    if len(parts) < 5 or parts[0] != owner or parts[1] != repo:
        raise GitHubHostError("Codeload path is not the pinned repository.")
    if parts[2] != "legacy.zip":
        raise GitHubHostError("Only official zip downloads are allowed.")
    if parts[3:5] != ["refs", "tags"] or len(parts) < 6:
        raise GitHubHostError("Codeload must be a release tag, not a branch.")
    tag = parts[5]
    if not is_release_tag(tag):
        raise GitHubHostError("Codeload tag is not an official release tag.")
    if expected_tag and tag != expected_tag.removeprefix("refs/tags/"):
        raise GitHubHostError("Codeload tag does not match the chosen release.")
    return raw


class _GitHubTransport(httpx.HTTPTransport):
    def __init__(self, expected_tag: str | None = None, **kwargs):
        super().__init__(**kwargs)
        self.expected_tag = expected_tag

    def handle_request(self, request: httpx.Request) -> httpx.Response:
        assert_github_overlay_url(str(request.url), self.expected_tag)
        return super().handle_request(request)


def github_request(
    method: str,
    url: str,
    *,
    expected_tag: str | None = None,
    timeout: float = 60.0,
    **kwargs,
) -> httpx.Response:
    assert_github_overlay_url(url, expected_tag)
    headers = dict(kwargs.pop("headers", None) or {})
    headers.setdefault("User-Agent", "SFTS-overlay/local")
    headers.pop("Authorization", None)
    headers.pop("authorization", None)
    with httpx.Client(
        transport=_GitHubTransport(expected_tag=expected_tag),
        follow_redirects=True,
        timeout=timeout,
        trust_env=False,
    ) as client:
        resp = client.request(method, url, headers=headers, **kwargs)
        assert_github_overlay_url(str(resp.url), expected_tag)
        return resp
