export function Select({ children }: { children?: React.ReactNode }) {
  return <select>{children}</select>
}

function Option({ value }: { value: string }) {
  return <option value={value} />
}
Option.displayName = 'Select.Option'

Select.Option = Option
