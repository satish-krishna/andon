import { FilterService, RangePreset } from '../app/core/filter.service';

export interface FilterFixtureOpts {
  range?: RangePreset;
  models?: string[];
  search?: string;
}

export function buildFilter(opts: FilterFixtureOpts = {}): FilterService {
  const f = new FilterService();
  if (opts.range) f.setRange(opts.range);
  if (opts.models) {
    f.allModels().forEach((m) => {
      const wanted = opts.models!.includes(m);
      const has = f.models().has(m);
      if (wanted !== has) f.toggleModel(m);
    });
  }
  if (opts.search != null) f.setSearch(opts.search);
  return f;
}
