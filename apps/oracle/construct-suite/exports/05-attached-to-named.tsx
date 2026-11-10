// construct: Tabs.Tab = Tab beside export function Tabs
// components: Tab=Tabs.Tab Tabs=Tabs
interface TabProps {
  p: 'tab';
}

interface TabsProps {
  p: 'tabs';
}

function Tab(props: TabProps) {
  return null;
}

export function Tabs(props: TabsProps) {
  return null;
}

Tabs.Tab = Tab;
