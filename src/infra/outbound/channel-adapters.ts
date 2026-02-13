import { Container, Separator, TextDisplay, type TopLevelComponents } from "@buape/carbon";
import type { ChannelId } from "../../channels/plugins/types.js";

export type CrossContextComponentsBuilder = (
  originLabel: string,
  message: string,
) => TopLevelComponents[];

export type ChannelMessageAdapter = {
  supportsComponentsV2: boolean;
  buildCrossContextComponents?: CrossContextComponentsBuilder;
};

type CrossContextContainerParams = {
  originLabel: string;
  message: string;
};

class CrossContextContainer extends Container {
  constructor({ originLabel, message }: CrossContextContainerParams) {
    const trimmed = message.trim();
    const components = [] as Array<TextDisplay | Separator>;
    if (trimmed) {
      components.push(new TextDisplay(message));
      components.push(new Separator({ divider: true, spacing: "small" }));
    }
    components.push(new TextDisplay(`*From ${originLabel}*`));
    super(components, { accentColor: "#5865F2" });
  }
}

const DEFAULT_ADAPTER: ChannelMessageAdapter = {
  supportsComponentsV2: false,
};

const DISCORD_ADAPTER: ChannelMessageAdapter = {
  supportsComponentsV2: true,
  buildCrossContextComponents: (originLabel: string, message: string) => [
    new CrossContextContainer({ originLabel, message }),
  ],
};

export function getChannelMessageAdapter(channel: ChannelId): ChannelMessageAdapter {
  if (channel === "discord") {
    return DISCORD_ADAPTER;
  }
  return DEFAULT_ADAPTER;
}
