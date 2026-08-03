# Revisión independiente — PR #197, ronda 2

La ronda 1 se detuvo en el ciclo 5 con bloqueantes vivos y `gate.js` salió 2,
como manda el contrato. Esta ronda arranca con el contador a cero, sobre el
árbol ya corregido, y por primera vez el gate corre con `--roles` de verdad:
`roles.js` sabe ahora registrar un implementador de fuera de la rotación
—Claude— y por tanto puede asignar árbitro sin conflicto (Codex).

El cambio se partió por áreas con `collect.js --area`, porque el diff completo
(≈400 KB) no cabe en un chat de navegador. Cada área es un paquete normal.

## Ciclo 1 — tres áreas, las tres bloqueadas

| Área | Qwen | GLM | Codex (árbitro) | Gate |
| --- | --- | --- | --- | --- |
| A · escritura y recolección | OBSERVACIONES | **BLOQUEANTE** | **BLOQUEANTE** | exit 1 |
| B · decisión (gate, roles, ask, build-prompt) | OBSERVACIONES | **BLOQUEANTE** | **BLOQUEANTE** | exit 1 |
| C · tests y documentación | OBSERVACIONES | OBSERVACIONES | **BLOQUEANTE** | exit 1 |

Trece defectos, todos reproducidos antes de corregirse y cada corrección
verificada por mutación. Los dos peores:

1. **Los cinco comandos se tragaban cualquier flag desconocido.** Se quedaban
   con lo que no empezara por `--`, así que `apply-files.js repo reply --chek`
   se leía como petición de comprobación y **hacía la escritura real**. La misma
   forma permitía tomar el valor de un flag por el posicional:
   `gate.js --cycle 1 --roles r.json review.md` juzgaba un fichero llamado «1».
2. **Toda la comprobación de identidad del gate estaba tras `if (ROLES)`.** La
   documentación decía obligatorio y el código decía opcional — y los tests
   llamaban al gate sin el flag exigiendo salida 0, de modo que la suite
   certificaba el modo silencioso en el que una revisión limpia escrita por el
   implementador cierra el ciclo. Es exactamente lo que esta skill existe para
   impedir, y estaba a un argumento olvidado de distancia.

## Ciclo 2 — incompleto, y por qué

| Área | Qwen | GLM |
| --- | --- | --- |
| A2 | (sesión atascada) | OBSERVACIONES |
| B2 | (sesión atascada) | **BLOQUEANTE** |
| C2 | (sesión atascada) | OBSERVACIONES |
| D2 | (sesión atascada) | OBSERVACIONES |

**Qwen no pudo completar el ciclo 2.** En dos intentos devolvió el propio
prompt en eco en las cuatro áreas. **z.ai llegó a servir un CAPTCHA** («drag the
slider to complete the puzzle») en una de las peticiones. No se intentó
sortearlo: es una medida antibot de la casa y rodearla no es una opción. Las
dos cuentas están dando señales de límite de uso.

Sin las dos revisiones ciegas no se lanzó el arbitraje de este ciclo, así que
**el ciclo 2 no está cerrado y el gate no ha vuelto a correr**. Queda anotado
como incompleto en vez de presentarlo como una vuelta terminada.

El bloqueante de GLM en B2 sí era real y está corregido: `roles.js` lanzaba
`ReferenceError` en vez del mensaje de error, porque el parser estricto llegó
mientras el texto seguía citando la variable anterior. Nada en la suite pasaba
un número de PR inválido, así que no lo detectó — lo detectó un revisor.

## Lo que esta ronda dice del propio orquestador

Tres defectos los introduje yo *durante* la ronda, y los tres son de la misma
familia que veníamos persiguiendo — una regla más estricta que aquello que
comprueba:

- exigir el veredicto en la primera línea hizo que se descartara toda revisión
  de navegador, porque los drivers imprimen su propia cabecera antes de la
  respuesta del modelo;
- exigirlo *exacto* descartó una revisión real de Qwen que escribe
  `VEREDICTO: BLOQUEANTE - <resumen>` en la misma línea;
- y `ask.js` toleraba espacios delante del veredicto mientras el gate no, de
  modo que una revisión válida se publicaba y luego el gate informaba de que no
  había veredicto.

Los tres se recuperaron gracias a los ficheros `.raw`, que existen precisamente
para eso.
