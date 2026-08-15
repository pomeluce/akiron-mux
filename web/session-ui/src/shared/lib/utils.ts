import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function basename(path: string) {
  const normalized = path.replace(/\/$/, '');
  return normalized.split('/').pop() || path;
}
