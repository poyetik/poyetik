/** The patch the playground plays: a 440 Hz sine on both channels. */
const PATCH = {
  format: 1,
  nodes: [
    { id: 1, box: "osc~", params: { frequency: 440 } },
    { id: 2, box: "dac~" },
  ],
  connections: [
    { from: [1, 0], to: [2, 0] },
    { from: [1, 0], to: [2, 1] },
  ],
};

/** The patch, as UTF-8 text in the patch format. */
export const SINE_440 = new TextEncoder().encode(JSON.stringify(PATCH));
