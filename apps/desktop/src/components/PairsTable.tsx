// Query parameters, headers and form fields: name, value, on/off. Typing in the empty last row
// adds a row; switched-off rows stay in the file but are not sent.
import { Pair } from "../api";

export function PairsTable({
  rows,
  onChange,
  namePlaceholder = "Имя",
  valuePlaceholder = "Значение",
}: {
  rows: Pair[];
  onChange: (rows: Pair[]) => void;
  namePlaceholder?: string;
  valuePlaceholder?: string;
}) {
  const all = [...rows, { name: "", value: "", enabled: true }];
  const set = (i: number, patch: Partial<Pair>) => {
    const next = all.map((r, j) => (j === i ? { ...r, ...patch } : r));
    onChange(next.filter((r, j) => j < rows.length || r.name || r.value));
  };
  return (
    <table className="pairs">
      <tbody>
        {all.map((row, i) => {
          const fresh = i === rows.length;
          return (
            <tr key={i} className={row.enabled ? "" : "off"}>
              <td className="chk">
                {!fresh && (
                  <input type="checkbox" checked={row.enabled} onChange={(e) => set(i, { enabled: e.target.checked })} title="Отправлять" />
                )}
              </td>
              <td>
                <input className="mono" value={row.name} placeholder={namePlaceholder} spellCheck={false} onChange={(e) => set(i, { name: e.target.value })} />
              </td>
              <td>
                <input className="mono" value={row.value} placeholder={valuePlaceholder} spellCheck={false} onChange={(e) => set(i, { value: e.target.value })} />
              </td>
              <td className="del">
                {!fresh && (
                  <button className="icon" title="Удалить" onClick={() => onChange(rows.filter((_, j) => j !== i))}>
                    ×
                  </button>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
