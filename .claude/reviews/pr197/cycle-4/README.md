# Revisión independiente — PR #197, ciclo 4

Área: `.claude/skills/multi-model-review/`, resultado integrado desde el
merge-base. El árbol no se modificó mientras los revisores trabajaban.

| Revisor | Modelo | Papel | Veredicto |
| --- | --- | --- | --- |
| Qwen | Qwen3.8-Max-Preview | ciego | OBSERVACIONES |
| GLM | GLM-5.2 | ciego | OBSERVACIONES |
| Codex | gpt-5.6-sol | árbitro | **BLOQUEANTE** |

Primera vuelta en que ningún revisor ciego bloquea, y primera en que el
arbitraje aporta bloqueantes que ninguno de los dos vio. Es exactamente para
lo que existe el tercer asiento.

Bloqueantes del árbitro, ambos corregidos:
 - `apply-files.js` validaba la contención con `path.resolve()`, que es
   textual: un symlink dentro del repo apuntando fuera pasaba la comprobación
   y `writeFileSync` lo seguía. Contradecía la garantía documentada.
 - El implementador no quedaba realmente fijo: el argumento de override
   permitía reasignarlo en un ciclo posterior, promoviendo al anterior a
   árbitro sobre un diff que contenía su propio trabajo. Ahora la asignación
   se persiste y una reasignación se rechaza.

Y una alta: el gate aceptaba cualquier fichero con las dos señales sin
comprobar de qué familia venía. `ask.js` registra ahora quién respondió y
`gate.js` exige que coincida con el árbitro asignado.

Nota sobre un hallazgo que no se sostuvo: GLM sostuvo que `git fetch origin
<base>` deja la referencia de seguimiento obsoleta. Medido, no es cierto — git
aplica igualmente el refspec configurado. El refspec explícito se mantiene por
claridad, no porque el otro estuviera roto.
