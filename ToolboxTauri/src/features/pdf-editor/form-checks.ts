import type { PdfFormEntry, PdfFormField, PdfFormFormat } from './contracts';

// Mirrors the native checks in kit/pdf/scene/forms.rs, which refuse the same entries on export.
const MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august', 'september', 'october', 'november', 'december'];

export function isRealDate(pattern: string, entry: string): boolean {
  const parts = pattern.match(/d+|m+|y+|[^dmy]/g) ?? [];
  const value = entry.trim();
  let at = 0; let day = 1; let month = 1; let year = 2000;
  const digits = (min: number, max: number) => {
    const found = /^\d+/.exec(value.slice(at, at + max))?.[0] ?? '';
    if (found.length < min) return null;
    at += found.length;
    return Number(found);
  };
  for (const part of parts) {
    let parsed: number | null;
    if (part === 'd' || part === 'dd') parsed = day = digits(part.length, 2) ?? NaN;
    else if (part === 'm' || part === 'mm') parsed = month = digits(part.length, 2) ?? NaN;
    else if (part === 'mmm' || part === 'mmmm') {
      const word = /^\p{L}*/u.exec(value.slice(at))![0];
      at += word.length;
      const index = MONTHS.findIndex((name) => (part === 'mmm' ? name.slice(0, 3) : name) === word.toLowerCase());
      parsed = month = index < 0 ? NaN : index + 1;
    } else if (part === 'yy' || part === 'yyyy') {
      const n = digits(part.length, part.length);
      parsed = year = n === null ? NaN : part === 'yyyy' ? n : n < 50 ? 2000 + n : 1900 + n;
    } else if (/[a-z0-9]/i.test(part)) return false;
    else parsed = value[at] === part ? (at += 1) : NaN;
    if (Number.isNaN(parsed)) return false;
  }
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return at === value.length && month >= 1 && month <= 12 && day >= 1 && day <= days[month - 1];
}

function isDecimal(entry: string, separator: number) {
  const mark = separator === 2 || separator === 3 ? ',' : '.';
  const [whole, fraction = '', ...rest] = entry.trim().replace(/^-/, '').split(mark);
  return rest.length === 0 && /^\d*$/.test(whole) && /^\d*$/.test(fraction) && whole.length + fraction.length > 0;
}

const SPECIAL = [
  { lengths: [5], flag: 'Not a ZIP code' },
  { lengths: [9], flag: 'Not a ZIP+4 code' },
  { lengths: [7, 10], flag: 'Not a phone number' },
  { lengths: [9], flag: 'Not a Social Security number' },
];

function formatFlag(format: PdfFormFormat, entry: string): string | null {
  switch (format.kind) {
    case 'date': return isRealDate(format.pattern, entry) ? null : 'Not a real date';
    case 'number': case 'percent': return isDecimal(entry, format.separator) ? null : 'Not a number';
    case 'special': {
      const special = SPECIAL[format.special];
      return /^[\d\s\-.()]*$/.test(entry) && special.lengths.includes(entry.replace(/\D/g, '').length) ? null : special.flag;
    }
  }
}

export function isFormEntryEmpty(value: PdfFormEntry) {
  return value === '' || value === false || (Array.isArray(value) && value.length === 0);
}

// A short flag for an entry the export would refuse, or null when it's fine.
export function formEntryFlag(field: PdfFormField, value: PdfFormEntry): string | null {
  if (field.type !== 'text' || typeof value !== 'string' || !value.trim() || !field.format) return null;
  return formatFlag(field.format, value);
}

// What the inspector shows for a filled choice field: the labels of the chosen options.
export function formChoiceText(field: PdfFormField, value: PdfFormEntry) {
  const label = (choice: string) => field.optionLabels[field.options.indexOf(choice)] ?? choice;
  return Array.isArray(value) ? value.map(label).join(', ') : label(String(value));
}
