// construct: export const Select = Object.assign(SelectRoot, { Option })
// components: Option=Select.Option Select=Select
interface OptionProps {
  p: 'option';
}

interface SelectProps {
  p: 'select';
}

function SelectRoot(props: SelectProps) {
  return null;
}

function Option(props: OptionProps) {
  return null;
}

export const Select = Object.assign(SelectRoot, { Option });
