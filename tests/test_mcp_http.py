from __future__ import annotations

import asyncio
import json
from collections.abc import Mapping

import httpx
import pytest
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client
from mcp.types import CallToolResult

from hieronymus.config import HieronymusConfig
from hieronymus.service_app import build_app
from hieronymus.service_state import ServerState

SERVICE_ORIGIN = "http://127.0.0.1:9768"
MCP_ENDPOINT = f"{SERVICE_ORIGIN}/mcp"


def _make_state(config: HieronymusConfig) -> ServerState:
    return ServerState(
        pid=12345,
        host="127.0.0.1",
        port=9768,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
    )


def _initialize_request() -> dict[str, object]:
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "hieronymus-tests", "version": "1"},
        },
    }


def _mcp_headers(origin: str | None = None) -> dict[str, str]:
    headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
    }
    if origin is not None:
        headers["Origin"] = origin
    return headers


def _text_payload(result: CallToolResult) -> object:
    content = result.content
    assert len(content) == 1
    return json.loads(content[0].text)


async def _exercise_protocol(
    config: HieronymusConfig,
    *,
    headers: Mapping[str, str] | None = None,
) -> None:
    app = build_app(config, _make_state(config))
    transport = httpx.ASGITransport(app=app)
    async with app.router.lifespan_context(app):
        async with httpx.AsyncClient(
            transport=transport,
            base_url=SERVICE_ORIGIN,
            headers=headers,
        ) as http_client:
            async with streamable_http_client(
                MCP_ENDPOINT,
                http_client=http_client,
            ) as (read_stream, write_stream, _):
                async with ClientSession(read_stream, write_stream) as session:
                    initialized = await session.initialize()
                    assert initialized.serverInfo.name == "hieronymus"

                    listed = await session.list_tools()
                    tool_names = {tool.name for tool in listed.tools}
                    assert "hieronymus_series_create" in tool_names
                    assert "hieronymus_series_list" in tool_names

                    created = await session.call_tool(
                        "hieronymus_series_create",
                        {"slug": "oso", "title": "Only Sense Online"},
                    )
                    assert created.isError is False
                    assert _text_payload(created) == {
                        "slug": "oso",
                        "title": "Only Sense Online",
                        "source_language": "",
                        "target_language": "",
                        "language_tags": [],
                        "id": 1,
                    }

                    series = await session.call_tool("hieronymus_series_list", {})
                    assert series.isError is False
                    assert _text_payload(series) == _text_payload(created)


def test_streamable_http_initializes_lists_and_calls_read_write_tools(
    config: HieronymusConfig,
) -> None:
    asyncio.run(_exercise_protocol(config))


def test_streamable_http_is_mounted_at_exact_mcp_path(config: HieronymusConfig) -> None:
    async def exercise() -> None:
        app = build_app(config, _make_state(config))
        async with app.router.lifespan_context(app):
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app=app),
                base_url=SERVICE_ORIGIN,
            ) as client:
                response = await client.post(
                    "/mcp/mcp",
                    json=_initialize_request(),
                    headers=_mcp_headers(),
                )
        assert response.status_code == 404

    asyncio.run(exercise())


@pytest.mark.parametrize(
    ("origin", "expected_status"),
    [
        (None, 200),
        (SERVICE_ORIGIN, 200),
        ("https://evil.example", 403),
    ],
)
def test_streamable_http_origin_boundary(
    config: HieronymusConfig,
    origin: str | None,
    expected_status: int,
) -> None:
    async def exercise() -> None:
        app = build_app(config, _make_state(config))
        async with app.router.lifespan_context(app):
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app=app),
                base_url=SERVICE_ORIGIN,
            ) as client:
                response = await client.post(
                    "/mcp",
                    json=_initialize_request(),
                    headers=_mcp_headers(origin),
                )
        assert response.status_code == expected_status

    asyncio.run(exercise())


def test_official_client_accepts_same_origin(config: HieronymusConfig) -> None:
    asyncio.run(_exercise_protocol(config, headers={"Origin": SERVICE_ORIGIN}))
