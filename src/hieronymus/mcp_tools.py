from __future__ import annotations

import inspect
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from functools import wraps
from typing import Any, Protocol

from mcp.server.fastmcp import FastMCP
from starlette.concurrency import run_in_threadpool

from hieronymus.config import HieronymusConfig

type ToolFunction = Callable[..., object]
type McpOperation = Callable[[HieronymusConfig, dict[str, object]], object]


class McpBackend(Protocol):
    def invoke(self, operation: str, params: dict[str, object]) -> object: ...


@dataclass(frozen=True)
class ToolContract:
    function: ToolFunction
    description: str | None = None

    @property
    def operation(self) -> str:
        return self.function.__name__.removeprefix("hieronymus_")


_TOOL_CONTRACTS: list[ToolContract] = []


def mcp_tool(*, description: str | None = None) -> Callable[[ToolFunction], ToolFunction]:
    def decorate(function: ToolFunction) -> ToolFunction:
        _TOOL_CONTRACTS.append(ToolContract(function, description))
        return function

    return decorate


def tool_contracts() -> tuple[ToolContract, ...]:
    return tuple(_TOOL_CONTRACTS)


def tool_operations() -> dict[str, ToolFunction]:
    return {contract.operation: contract.function for contract in _TOOL_CONTRACTS}


class DirectMcpBackend:
    def __init__(
        self,
        config: HieronymusConfig,
        operations: Mapping[str, McpOperation],
    ) -> None:
        self._config = config
        self._operations = operations

    def invoke(self, operation: str, params: dict[str, object]) -> object:
        return self._operations[operation](self._config, params)


class ProxyMcpBackend:
    def __init__(self, client_factory: Callable[[], Any]) -> None:
        self._client_factory = client_factory

    def invoke(self, operation: str, params: dict[str, object]) -> object:
        return self._client_factory().invoke(operation, params)


def _backend_tool(contract: ToolContract, backend: McpBackend) -> ToolFunction:
    signature = inspect.signature(contract.function)

    @wraps(contract.function)
    async def invoke(*args: Any, **kwargs: Any) -> object:
        bound = signature.bind(*args, **kwargs)
        bound.apply_defaults()
        return await run_in_threadpool(
            backend.invoke,
            contract.operation,
            dict(bound.arguments),
        )

    return invoke


def create_mcp_server(
    backend: McpBackend,
    *,
    streamable_http_path: str = "/mcp",
    json_response: bool = False,
    stateless_http: bool = False,
) -> FastMCP:
    server = FastMCP(
        "hieronymus",
        streamable_http_path=streamable_http_path,
        json_response=json_response,
        stateless_http=stateless_http,
    )
    for contract in tool_contracts():
        server.tool(description=contract.description)(_backend_tool(contract, backend))
    return server
