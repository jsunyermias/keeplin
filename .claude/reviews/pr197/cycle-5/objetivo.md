PR jsunyermias/keeplin#197, área `.claude/skills/multi-model-review/`, CICLO 5.

Objetivo: orquestar la revisión independiente que exige AGENTS.md a través de
varias familias de modelo, moviendo diffs y revisiones como ficheros para que
casi nada pase por el contexto del agente que orquesta.

AVISO IMPORTANTE: este es el último ciclo antes del tope. Si vuelve a bloquear,
el gate sale con código 2 y el diseño obliga a parar y llevarlo al mantenedor
en lugar de seguir girando.

HISTORIA. Los ciclos 1 a 4 bloquearon. El 4 fue el primero en que ningún
revisor ciego bloqueó y el arbitraje aportó lo que ninguno vio. Correcciones
del ciclo 4, presentes en este diff:
 - apply-files.js: la contención se comprobaba con path.resolve(), que es
   textual, así que un symlink dentro del repo apuntando fuera pasaba y
   writeFileSync lo seguía. Ahora se resuelve el realpath del repo y del
   ancestro existente más profundo de cada destino.
 - roles.js: el implementador no quedaba realmente fijo; el override permitía
   reasignarlo en un ciclo posterior, ascendiendo al anterior a árbitro sobre
   un diff con su propio trabajo. La asignación se persiste y una reasignación
   contradictoria se rechaza.
 - ask.js registra qué familia respondió; gate.js con --roles exige que la
   revisión venga del árbitro asignado y rechaza la del implementador.
 - Los drivers fallan si la respuesta extraída viene vacía, en vez de
   registrarla como ejecución correcta.
 - 24 tests offline. Las protecciones nuevas verificadas por mutación.

NOTA DE HONESTIDAD sobre un hallazgo previo: un revisor sostuvo que
`git fetch origin <base>` deja obsoleta la referencia de seguimiento. Se midió
y NO es cierto: git aplica igualmente el refspec configurado. El refspec
explícito se mantiene por claridad, y el comentario lo dice así.

Tu tarea: verificar si estas correcciones resuelven lo señalado y si han
introducido problemas nuevos. El diff es el resultado integrado completo desde
el merge-base.
