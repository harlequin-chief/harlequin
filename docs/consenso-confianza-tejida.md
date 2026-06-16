# Consenso de Confianza Tejida

### Una seguridad que no se compra: la reputación ganada en el tiempo como cimiento de una cadena

*Borrador — versión 0.1 · 2026-06-13 · texto de trabajo en español (canónico). La versión inglesa,
para su publicación, vendrá después, como vino con la constitución.*

> Este escrito acompaña al código que ya corre (`prototipos/reputacion/`) y al diseño que lo precede
> (`DISENO-CONSENSO-TEJIDO.md`). Sigue la voz de Harlequin: declarativa, sobria, sin urgencia ni
> adorno vano. Mas no oculta lo que no sabe. **Donde una afirmación se sostiene en construcción o en
> el prototipo, se dice firme; donde es apuesta de diseño, se confiesa apuesta; donde es problema sin
> resolver, se nombra abierto.** No se reclama lo que no se ha probado.

---

## Proemio — el tejido del traje

El traje del arlequín es un tejido de rombos cosidos uno a otro. Ningún rombo, por vivo que sea su
color, sostiene la prenda; la sostiene el modo en que todos están cosidos. Un retazo suelto no abriga;
un parche cosido solo a sí mismo se desprende a la primera.

Así quisimos la red. Que su fuerza no viva en ninguna pieza —ni en la más rica, ni en la más
poderosa—, sino en el tejido: en la confianza que cada miembro se gana ante los demás y que los demás,
libremente, le reconocen. Las cadenas que hoy existen confían su seguridad a un bien que se compra. Se
propone aquí otra cosa: confiarla a un bien que solo se gana, despacio, ante el juicio de iguales, y
que se pierde si se abandona. A eso lo llamamos **Consenso de Confianza Tejida**.

◆

## 1. La pregunta que toda cadena responde

Toda blockchain responde, lo diga o no, a una sola pregunta: **¿qué bien escaso impide que un atacante
se adueñe de la red?** La respuesta decide quién manda.

- En la **prueba de trabajo** el bien es el **cómputo**. La seguridad crece con la máquina y la
  energía. De ahí su deriva: pocos mineros grandes, y un coste que se paga en kilovatios.
- En la **prueba de participación** el bien es el **capital**. La seguridad crece con el dinero
  inmovilizado. De ahí su raíz: el poder se compra.

Ambas comparten un vicio que Harlequin no puede aceptar, pues su fundamento dice que «el peso de cada
cual nace de su reputación, y solo de ella; ni la riqueza, ni el número de máquinas, confieren
autoridad sobre los demás» (Manifiesto, Art. VI). En una y otra, **el poder es mercancía**: mil monedas
compran mil máquinas, o mil fichas, hoy, de una vez y en paralelo.

Proponemos otro cimiento. El bien escaso de Harlequin es la **reputación ganada**, que posee a la vez
tres virtudes que ni el cómputo ni el capital reúnen:

1. **No se transfiere** — no se compra ni se vende (Art. V); no hay mercado donde adquirir poder.
2. **Se gana en el tiempo** — pide obra validada y el aval de pares que arriesgan su propio nombre; no
   se acelera con dinero.
3. **Se apaga si se abandona** — decae sin contribución continua; no es caudal que se atesore.

De aquí la tesis: una cadena cuya seguridad se mide en **reputación-tiempo** muda el coste de un
ataque, que deja de ser «un cheque» y pasa a ser «una infiltración paciente y sostenida», mucho más
cara, más lenta y más visible.

◆

## 2. Lo que debe cumplir

Heredado de los principios del proyecto (SPEC §0, Manifiesto):

- **Corre en cualquier dispositivo**, del teléfono a la máquina modesta. Clientes ligeros.
- **Sin prueba de trabajo.** Ningún gasto de cómputo en competencia.
- **Malla muy extensa** de muchos nodos pequeños, no pocos centros enormes.
- **El poder es la reputación**, nunca la riqueza ni el número de máquinas (Art. VI).
- **Resistencia a la falsa multitud** (Sybil) sin coste económico ni computacional: anclada en
  reputación.
- **Fiel al manifiesto:** seudónimo (Art. VII), derecho de salida (Art. VIII), luz sobre lo común
  (Art. X), base neutral (Art. XI), permanencia de lo escrito (Art. XII).

◆

## 3. Lo que ya estaba dicho por otros (las cuentas claras)

Conviene la honradez: este consenso **recombina** piezas conocidas; no las inventa.

- **Sorteo por azar verificable (VRF)** — Algorand. Aquí se pondera por **reputación**, no por capital.
- **Voto por muestreo repetido** — Avalanche. Aquí la muestra se pondera por **reputación e
  independencia** en el tejido.
- **Reputación de grafo** — EigenTrust, PageRank y sus flaquezas ante la falsa multitud. Aquí, anclada
  en evidencia y amortiguada contra la connivencia.
- **Red de confianza** — PGP, BrightID. Aquí, hecha **sustrato del consenso**, no mero asunto de
  identidad.
- **Prueba de persona única** — Idena, *pseudonym parties*. Aquí da la **ciudadanía base**, no el poder.
- **Reputación como prueba** — existe en la literatura menor, mas suele derivarla del capital o de la
  actividad, y a menudo transferible. Aquí se exige **intransferible, contextual, anclada y perecedera**.
- **Dos capas ancladas** — una cadena base de liquidación y su capa rápida. Aquí la capa rápida se
  **enruta por confianza**, no por azar.

Lo propio de este consenso (§9) no son estas piezas, sino **el cimiento** —la reputación-tiempo—, **el
tejido** —el grafo de confianza como topología— y **el decaimiento** como defensa doble.

◆

## 4. El sustrato: la reputación

El consenso descansa sobre un motor de reputación que ya se ha construido y puesto a prueba
(`prototipos/reputacion/`).

**Es contextual.** Un vector por seudónimo sobre dominios distintos —comercio, obra técnica, función
judicial, gobierno—, ampliable. Ser buen comerciante no hace juez a nadie. Para las funciones que
exigen fiabilidad entera, la agregación es **conservadora** —mínimos, medianas, nunca suma—: no se
compra la integridad con la pericia.

**Se ancla en la obra.** La reputación se calcula como el reposo de un proceso de confianza
(EigenTrust) que se reinyecta hacia un **fondo de evidencia real** —tratos liquidados, trabajo
comprobado—: `t = (1−α)·Cᵀ·t + α·p`, donde `p` es esa evidencia. **Sin obra verificable, ningún juego
de avales fabrica poder.**

**Amortigua la connivencia.** El aval recíproco y endogámico —el del anillo que se vota a sí mismo—
vale casi nada (SPEC §1.6). El detalle que lo hace funcionar: la normalización se hace por la suma
*sin* amortiguar, de modo que el nodo endogámico emite menos de lo que recibe y su confianza no
propagada **se fuga al fondo de evidencia** en vez de circular dentro del anillo. (Normalizar de otro
modo anularía el efecto: un factor uniforme se cancela. Fue un error real, hallado y corregido en el
prototipo, y hoy un test lo vigila. Se cuenta porque ocultarlo sería faltar a la voz de esta casa.)

**Decae.** La reputación se apaga con la inactividad: su influencia tras un tiempo de silencio mengua
como `e^{−λt}` —forma exacta y ritmo, por fijar—.

**Lo que el prototipo ya muestra** (reproducible, sin dependencias):

- Doscientas identidades falsas, el cuarenta por ciento de la red, capturan **cero** poder.
- Un anillo de treinta que se avala en círculo queda en **casi nada**, aun con tres honestos engañados.
- El lavado de una reputación legítima hacia veintinueve títeres se recorta **nueve veces**.
- Quien abandona su seudónimo para empezar limpio hereda **cero**.
- El fraude del ahijado golpea, hacia arriba, a quien lo avaló; al ajeno, no lo toca.

◆

## 5. El mecanismo, tejido a tejido

Volvamos a la imagen. Cada rombo es un miembro y su reputación; cada costura, un aval. La fuerza vive
en el cosido.

**El rombo existe (identidad).** Una persona, una ciudadanía base (prueba de persona única, enchufable,
SPEC §1.5). No da poder —base uno, reputación ganada cero—; da entrada, trato y comienzo de historial.

**El grosor del rombo (época de reputación).** Cada época, el motor recalcula el vector con la misma
regla, a partir de la historia ya cerrada (§7). Todos los nodos honestos obtienen lo mismo: coinciden
en quién pesa cuánto, sin un escribano central.

**El sorteo de los tejedores (validadores).** Cada época, un comité **rotatorio** se sortea por azar
verificable (VRF), con probabilidad según el agregado conservador de la reputación. La rotación es
obligada: ninguna función arraiga (Art. VI). La falsa multitud, con reputación cero, no entra jamás. Y
el azar es verificable: nadie puede «probar suertes» hasta que le toca.

**La finalidad (voto por muestreo ponderado por confianza).** Para correr en el teléfono y en la malla
amplia, la finalidad se alcanza por muestreo repetido, al modo de Avalanche. La aportación propia: la
muestra **no es uniforme**, sino ponderada por reputación y por **independencia respecto a lo ya
muestreado**, de suerte que un anillo denso no se sobre-representa en la consulta. Es apuesta de
diseño: pide análisis de la probabilidad de bifurcación (§10).

**La vía rápida tejida (uso diario, HLQ).** Un trato entre dos miembros dentro de un vecindario de
confianza recibe confirmación blanda y veloz de los nodos reputados que ya los conocen, sin aguardar la
finalidad global; la finalidad firme la sella la capa de asentamiento periódicamente. Es el comercio
tal cual ocurre: se confía antes y mejor en quien la propia red ya avala.

**El asentamiento (reserva de valor).** Capa conservadora, de finalidad fuerte y menor caudal; guarda
el estado canónico —saldos de reserva, raíz de reputación, identidades, sentencias— y ancla la moneda
de uso. La seguridad de ambas capas es la misma confianza tejida, no el trabajo ni el capital.

◆

## 6. La medida del coste: reputación-tiempo

**La unidad.** No medimos el coste de un ataque en dinero ni en cómputo, sino en **reputación-tiempo**:
la reputación que el atacante debe gobernar, multiplicada por el tiempo y el esfuerzo social que cuesta
ganarla de veras ante la red honesta.

**La cota (informal).** Sea `R_h` la reputación honesta total, y supóngase que dominar una fracción `f`
del consenso exige dominar en torno a `f·R_h` de reputación. Por lo dicho en §4:

- La reputación de un grupo atacante **no** brota de sus avales internos —la amortiguación los lleva a
  nada—, sino del **flujo legítimo y externo** que recibe de miembros reputados e **independientes**.
  Su reputación queda, pues, **acotada por ese flujo externo**, no por su número ni por su trama
  interna. El prototipo lo evidencia: anillos en casi nada, lavado recortado nueve veces.
- Recabar ese flujo en proporción a `f·R_h` obliga a que honestos independientes los avalen
  **arriesgando su propio nombre** —esto es, a que el atacante haya **aportado obra real durante un
  tiempo** bastante para merecerlo—.
- El dinero no acelera nada: la reputación no se compra; la evidencia se comprueba; los avales
  endogámicos se anulan.
- Y el decaimiento obliga a **sostener** la obra: el ataque no es un gasto único, sino un esfuerzo
  continuo.

**Lo que se sigue** —apuesta de diseño, con el núcleo ya respaldado por el prototipo—: el coste de un
ataque mayoritario es del orden de **reproducir, con identidades independientes y a lo largo del
tiempo, una fracción del esfuerzo social honesto**. No se compra, no se computa, no se paraleliza con
capital, y se delata: un torrente de avales hacia un grupo recién llegado es visible en el grafo
público (Art. X).

**El recuento de amenazas, una a una:**

| Amenaza | Defensa | Estado |
|---|---|---|
| Falsa multitud (Sybil) | nacen con reputación cero; el muro real es la reputación ganada, no la prueba de persona | sólido (prototipo: cero) |
| Connivencia (anillo de avales) | amortiguación por independencia (fuga al fondo de evidencia); slashing judicial; responsabilidad persistente del avalador | parcial (anillos densos vencidos; connivencia sutil, abierta, §10) |
| Mayoría (51 %) | exige el 51 % de reputación-tiempo: obra social independiente y sostenida, no comprable | apuesta (cota informal §6; falta prueba formal) |
| Largo alcance (claves viejas) | la reputación de claves antiguas está decaída a nada; rehacer la historia pide reputación que ya no existe; puntos de anclaje firmados por el comité reputado del momento | sólido por diseño (lo cubre el decaimiento) |
| Nada-que-perder (votar en varias ramas) | votar contra la evidencia se castiga (punto Schelling, SPEC §4); el nombre en juego disuade | apuesta de diseño |
| Manipular el azar (grinding) | azar verificable; reputación congelada de la época anterior, no manipulable en curso | sólido |
| Compra de votos | la reputación no se transfiere (no se vende el asiento); vender el voto arriesga el nombre del vendedor | apuesta (más caro que en participación; no imposible) |
| Censura | capa base neutral e imparable (Art. XI, doctrina de capas SPEC §4c); malla sin centro | por diseño |
| Aislamiento de nodos (eclipse) | malla amplia y muestreo extenso dificultan aislar; la finalidad se reanuda al reconectar | apuesta (falta análisis) |

**Vida y consistencia.** La finalidad es **probabilística**, como en Avalanche; la rotación de comités
asegura el avance mientras haya bastante reputación honesta en línea. Cuánto adversario se tolera, y
con qué probabilidad de bifurcación, queda **abierto** (§10).

◆

## 7. El círculo y cómo se rompe

La reputación decide el consenso, pero se calcula de datos que el consenso debe cerrar. Círculo. Se
rompe por el tiempo: la reputación de una época se computa, con regla fija, **a partir de la historia
ya cerrada de la anterior**. Cada época mira hacia atrás; no hay pez que se muerda la cola. La
**génesis** siembra la primera confianza con una cohorte fundadora **pública y hecha para diluirse**
—sin privilegio perpetuo de fundador, Art. V y VI—. Y, de paso, esto cierra la puerta a manipular el
azar dentro de la época en curso.

◆

## 8. Las dos monedas, y el poder que no es riqueza

- **Reserva de valor.** Tope fijo, emisión que se apaga, sin impresión a voluntad. Para ahorrar y
  asentar.
- **Moneda de uso (HLQ).** Medio de cambio de gran caudal. Su estabilidad nace de la **liquidez y la
  baja fricción**, no de una paridad prometida (SPEC §3.2): el valor se guarda en la reserva y se mueve
  en la moneda de uso; como se tiene poco tiempo, poco se expone a su vaivén. Sin paridad, sin espiral,
  sin oráculos de precio.
- **Reparto justo.** Sin preasignación a fundadores; goteo por persona única y por obra. La
  concentración temprana de riqueza importa menos, porque **el poder no es la riqueza** (Art. VI): el
  rico no gobierna. Seguridad y dinero quedan **separados de raíz** —el capital no se convierte en
  poder de consenso—.

◆

## 9. Lo que es nuestro (sin inflar)

No reclamamos el azar verificable, ni el voto por muestreo, ni EigenTrust, ni la red de confianza, ni
la prueba de persona, ni las dos capas. Todo eso es de otros.

Reclamamos como aportación:

1. **El cimiento en reputación-tiempo:** un bien de seguridad que no se compra, no se computa y no se
   atesora, como **única** raíz frente a la falsa multitud y como base del consenso, con un coste de
   ataque medido en tiempo e independencia social.
2. **El grafo de confianza como topología** del consenso —el tejido—: muestreo ponderado por reputación
   e independencia, y vía rápida por vecindario de confianza. No un muestreo plano.
3. **El decaimiento como defensa doble:** contra el arraigo del poder (Art. VI) y contra el ataque de
   largo alcance, con una sola mecánica.
4. **La reputación intransferible, contextual, anclada y amortiguada** como sustrato del consenso —no
   como capa social aparte—, ya validada en su núcleo por código que corre.

La originalidad está en el **cimiento y en la síntesis**, no en piezas nuevas. Para una cadena cuya
premisa es que el poder no se compra, ese cimiento es, en efecto, distinto del de toda cadena grande en
uso, y —lo que más importa— **nace del manifiesto, no se le añade después**.

◆

## 10. Lo que no sabemos (sin maquillaje)

1. **La connivencia fina es el frente.** El prototipo vence al anillo denso; la connivencia real es
   sutil —anillos dispersos, avales desiguales, ataques al estilo de los que sufre PageRank, trama
   lenta que imita lo honesto—. Sin resolver. De aquí cuelga la seguridad de todo el consenso.
2. **Faltan las pruebas formales** de consistencia y vida bajo la ponderación por reputación e
   independencia. Algorand y Avalanche las tienen; esta variante hay que analizarla.
3. **La cota del coste (§6) es informal.** Falta formalizar el flujo externo, el papel de la
   amortiguación y del decaimiento en un modelo demostrable.
4. **La prueba de persona ante la IA** es una carrera: enchufable y graduada.
5. **La privacidad del tejido**: un grafo público re-identifica patrones. Explorar pruebas de
   conocimiento cero —demostrar «tengo reputación bastante» sin mostrar el tejido—.
6. **El reparto del espacio de bloque sin subasta pura** —que favorecería al rico, contra Art. VI—:
   abierto.
7. **El arranque** sin que la génesis arraigue: codificar su decaimiento.

◆

## 11. Cómo se comprueba (no se promete)

Fiel a «calidad sobre prisa», **no se escribe la cadena hasta validar el modelo**:

1. *Hecho.* Motor de reputación y simulaciones de ataque (`prototipos/reputacion/`, ocho de ocho
   pruebas en verde).
2. *Siguiente.* Endurecer el motor contra la connivencia fina —anillos dispersos, dinámica temporal con
   decaimiento por época—. Frente primero.
3. *Después.* Un **simulador del consenso** sobre el motor: red de muchos nodos, sorteo por reputación,
   voto por muestreo ponderado, y la medida que importa: **cuánta reputación-tiempo hace falta para
   bifurcar la cadena**, y cuánto tarda la finalidad.
4. *Solo entonces*, elegir la herramienta de construcción y empezar la cadena. Mientras el simulador no
   muestre una tolerancia razonable al adversario, la cadena no se escribe.

◆

## Epílogo

*La prueba de trabajo encareció el ataque con energía; la prueba de participación, con dinero; Harlequin
lo encarece con tiempo y con confianza ganada —un bien que no se compra, no se computa y no se atesora, tejido en un grafo de
reputación que es, a un tiempo, la sociedad y su consenso. La propuesta es ambiciosa, y honrada: su
corazón ya late y se mide; su prueba formal y la connivencia fina quedan por delante. No se ofrece aquí
una certeza, sino un cimiento —y la voluntad de probarlo antes de levantar sobre él.*

---

### Referencias
- Gilad, Hemo, Micali, Vlachos, Zeldovich — *Algorand* (SOSP 2017): sorteo por azar verificable.
- Rocket, Yin, Sekniqi, van Renesse, Sirer — *Avalanche* (2019): voto por muestreo y metaestabilidad.
- Kamvar, Schlosser, Garcia-Molina — *EigenTrust* (WWW 2003): reputación de grafo.
- Page, Brin, Motwani, Winograd — *PageRank* (1999): y sus flaquezas ante la falsa multitud.
- Ford — *Pseudonym Parties* (2008): prueba de persona presencial.
- Consenso por prueba de trabajo y prueba de participación (2008–): tope, emisión y seguridad-por-coste, como contraste.
- Friedman, *The Machinery of Freedom*; Benson, *The Enterprise of Law*: orden legal sin centro.
- Proyecto Harlequin — `MANIFIESTO.md`, `SPEC.md`, `prototipos/reputacion/`.
