// A dependency's condition maps, as Chakra's `ConditionalValue<V>` writes them: one map per value type, every one over
// the same 45 keys (past the 40 an object below a prop may list), written inside a union so no alias names the map.
export interface Conditions {
  c01?: boolean;
  c02?: boolean;
  c03?: boolean;
  c04?: boolean;
  c05?: boolean;
  c06?: boolean;
  c07?: boolean;
  c08?: boolean;
  c09?: boolean;
  c10?: boolean;
  c11?: boolean;
  c12?: boolean;
  c13?: boolean;
  c14?: boolean;
  c15?: boolean;
  c16?: boolean;
  c17?: boolean;
  c18?: boolean;
  c19?: boolean;
  c20?: boolean;
  c21?: boolean;
  c22?: boolean;
  c23?: boolean;
  c24?: boolean;
  c25?: boolean;
  c26?: boolean;
  c27?: boolean;
  c28?: boolean;
  c29?: boolean;
  c30?: boolean;
  c31?: boolean;
  c32?: boolean;
  c33?: boolean;
  c34?: boolean;
  c35?: boolean;
  c36?: boolean;
  c37?: boolean;
  c38?: boolean;
  c39?: boolean;
  c40?: boolean;
  c41?: boolean;
  c42?: boolean;
  c43?: boolean;
  c44?: boolean;
  c45?: boolean;
}

export interface Little {
  l1?: boolean;
  l2?: boolean;
  l3?: boolean;
}

export type Value<V> = V | { [K in keyof Conditions]?: V };

export type Few<V> = V | { [K in keyof Little]?: V };

// The same 45 keys as the project's `Ours`: a map over a type parameter has the provenance of the type it is given.
export interface Theirs {
  k01?: boolean;
  k02?: boolean;
  k03?: boolean;
  k04?: boolean;
  k05?: boolean;
  k06?: boolean;
  k07?: boolean;
  k08?: boolean;
  k09?: boolean;
  k10?: boolean;
  k11?: boolean;
  k12?: boolean;
  k13?: boolean;
  k14?: boolean;
  k15?: boolean;
  k16?: boolean;
  k17?: boolean;
  k18?: boolean;
  k19?: boolean;
  k20?: boolean;
  k21?: boolean;
  k22?: boolean;
  k23?: boolean;
  k24?: boolean;
  k25?: boolean;
  k26?: boolean;
  k27?: boolean;
  k28?: boolean;
  k29?: boolean;
  k30?: boolean;
  k31?: boolean;
  k32?: boolean;
  k33?: boolean;
  k34?: boolean;
  k35?: boolean;
  k36?: boolean;
  k37?: boolean;
  k38?: boolean;
  k39?: boolean;
  k40?: boolean;
  k41?: boolean;
  k42?: boolean;
  k43?: boolean;
  k44?: boolean;
  k45?: boolean;
}
