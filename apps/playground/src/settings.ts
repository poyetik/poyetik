/** Settings of the playground. */

/** Gain before the speakers: a full-scale sine is loud. About -12 dB. */
export const GAIN = 0.25;

/**
 * Duration kept for the oscilloscope and the measurements, in seconds: 85 ms at every
 * sample rate. With a 10 ms window, a rising crossing to trigger on is found for every
 * period up to 75 ms (13.4 Hz and above). The frequency, measured over at least three
 * periods, is shown at every phase from about 47.1 Hz (47.13 Hz at 8 kHz: four periods in
 * the history after its first sample; tested), and never at 35.29 Hz (three periods per
 * history) or below.
 */
export const HISTORY_SECONDS = 0.085;

/**
 * Duration of the fade in and out, in seconds: 20 ms. Starting or stopping a sine at full
 * level is a step, heard as a click; a linear ramp of 5 to 20 ms removes it, and 20 ms
 * spans a whole cycle down to 50 Hz while still feeling instantaneous.
 */
export const FADE = 0.02;
