export type SpaceScale = '1' | '2' | '3' | '4' | '5' | '6' | '8';
export type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';
export type ThemeMode = 'light' | 'dark';

export const SPACE_CLASS: Record<SpaceScale, string> = {
  '1': 'fm-gap-1',
  '2': 'fm-gap-2',
  '3': 'fm-gap-3',
  '4': 'fm-gap-4',
  '5': 'fm-gap-5',
  '6': 'fm-gap-6',
  '8': 'fm-gap-8',
};
