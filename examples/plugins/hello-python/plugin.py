"""hello-python: a hanafubuki-agent plugin.

The Agent starts the exported program and calls it over the guest protocol
(kardia-guest/2) on its stdin and stdout; pykardia's ``serve`` sends
``print`` to stderr.
"""

from __future__ import annotations

from dataclasses import dataclass

from pykardia.guest import Error, Guest, Host, JsonValue, Meta, Request
from typing_extensions import Self


@dataclass
class Settings:
    """Its settings (``configSchema`` in the manifest), ``{}`` until some are set."""

    unit: str = "words"


class Plugin(Guest[Settings]):
    """A tool that counts words, and a prompt fragment that offers it."""

    Config = Settings

    def __init__(self, settings: Settings) -> None:
        """Keep the settings it started with."""
        self.settings = settings

    @staticmethod
    def meta() -> Meta:
        """Declare the manifest's name, providing the one service the Agent calls."""
        return Meta.plugin("hello-python").provide("plugin")

    @classmethod
    def apply(cls, host: Host, config: Settings, imports: dict[str, JsonValue]) -> Self:
        """Provide the service and start with the settings."""
        host.provide("plugin", {})
        return cls(config)

    def call(self, host: Host, request: Request) -> JsonValue:
        """Answer the Agent: one method per kind of contribution the manifest declares."""
        args = request.args
        if not isinstance(args, dict):
            raise Error("the Agent sends an object")
        if request.method == "tool":
            return self.tool(args)
        if request.method == "prompt":
            return "Count the words of a text with the hello-python plugin's count_words tool."
        return super().call(host, request)

    def tool(self, args: dict[str, JsonValue]) -> JsonValue:
        """Run a tool: a string is its text, any other value structured content."""
        arguments = args.get("arguments")
        if args.get("name") != "count_words" or not isinstance(arguments, dict):
            raise Error(f"no tool {args.get('name')}")
        text = arguments.get("text")
        if not isinstance(text, str):
            raise Error("text is a string")
        return {self.settings.unit: len(text.split())}
