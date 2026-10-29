// The password rule, checked in the browser before sending; the server checks it again.

/** 8 to 256 characters, with an uppercase letter, a lowercase letter and one that is not a letter or digit. */
export function passwordOk(p: string): boolean {
  const n = [...p].length;
  return (
    n >= 8 &&
    n <= 256 &&
    /\p{Lowercase}/u.test(p) &&
    /\p{Uppercase}/u.test(p) &&
    /[^\p{Alphabetic}\p{N}]/u.test(p)
  );
}
