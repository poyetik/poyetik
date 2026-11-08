/** Settings of the playground. */

/** Gain before the speakers: a full-scale sine is loud. About -12 dB. */
export const GAIN = 0.25;

/** Number of samples kept for the oscilloscope and the measurements: about 85 ms at 48 kHz. */
export const HISTORY = 4096;

/**
 * Duration of the fade in and out, in seconds: 20 ms. Starting or stopping a sine at full
 * level is a step, heard as a click; a linear ramp of 5 to 20 ms removes it, and 20 ms
 * spans a whole cycle down to 50 Hz while still feeling instantaneous.
 */
export const FADE = 0.02;
