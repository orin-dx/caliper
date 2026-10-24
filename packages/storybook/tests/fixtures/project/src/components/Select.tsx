export function Select({ children }: { children?: React.ReactNode }) {
  return <select>{children}</select>
}

Select.Option = function Option({ value }: { value: string }) {
  return <option value={value} />
}
