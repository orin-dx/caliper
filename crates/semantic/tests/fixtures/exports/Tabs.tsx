export interface TabsProps {
  selected: string;
}

export interface TabProps {
  id: string;
}

function TabsRoot(props: TabsProps) {
  return null;
}

function Tab(props: TabProps) {
  return null;
}

export const Tabs = Object.assign(TabsRoot, { Tab });
