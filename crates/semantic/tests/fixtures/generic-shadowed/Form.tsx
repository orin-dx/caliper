type FormData = { id: string };

/** Its default is a type of this module whose name the global scope also declares, in the default DOM libraries. */
export function Form<T = FormData>(props: { value: T }) {
  return null;
}
