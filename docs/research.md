# Research basis

Published work that shapes StochCraft's roadmap: what each source says that matters here, and
what StochCraft does about it. The summaries are StochCraft's own. The documents themselves are
not in the repository; follow the references.

The roadmap items they lead to are in [ROADMAP.md](../ROADMAP.md) under M1.5, M2.8 and "After
M3".

## Simulation credibility

### NASA-STD-7009B, Standard for Models and Simulations (2024)

NASA's standard for using models and simulations in decisions. Most of its requirements ask for a
*record*: of the intended and permissible uses, the assumptions, the limits and the domains of
verification and validation, the inputs and their pedigree, the warnings and errors seen while
running, how uncertainty was characterised, the sensitivity analyses done, and the risks of
acting on the results. Results reported to decision makers must carry explicit warnings (unmet
acceptance criteria, violated assumptions or limits, execution warnings) and an estimate of their
uncertainty with the method used to get it ([M&S 32]–[M&S 34]). A model used outside its
permissible uses or domains of validation must be *placarded* with which limit was exceeded, by
how much and with what consequence ([M&S 26]). Recommended practice includes version-controlled
models and data, test cases that span the model's limits, failure messages that name the point of
failure, and a way for users to report unusual results.

**For StochCraft.** Much of this is bookkeeping a simulator can do for its user:

| Requirement | StochCraft |
|---|---|
| [M&S 24] inputs and their pedigree | Each parameter can record where it came from (data set, expert elicitation, assumption), as a `stochcraft:` BPSim vendor extension |
| [M&S 25], 4.3.2 b, f: set-up, software version, versions of results | Every result carries a run record: StochCraft version, model digest, scenario, seed, replications, the journal of commands (M1.5) |
| [M&S 27], [M&S 32]: warnings and errors | Diagnostics and file notes travel with the results |
| [M&S 26]: use outside the validated domain | A model can declare validated ranges for its parameters; results from outside them are placarded |
| [M&S 30], [M&S 33], [M&S 34]: sensitivity, uncertainty and its method | Sweeps record their sensitivity results; results state their interval method |
| 4.2.2 e: test cases spanning the limits | The queueing-theory tests and the golden models (M1.6) |

A `report` command can then assemble a results assessment in the standard's terms.

> NASA Office of the Chief Engineer, *NASA-STD-7009B: Standard for Models and Simulations*,
> approved 2024-03-05, superseding NASA-STD-7009A.

## Discrete-event simulation in practice

### Facility sizing for cost estimates (NNSA, 2022)

The National Nuclear Security Administration sizes facilities for cost estimates with DES:
process times from three-point (minimum, mode, maximum) estimates, shared equipment, working hours
(weekdays 8 to 5), a production target per week, and a confidence level, the share of simulated
weeks that meet the target (70% in the worked example). The equipment list is found by running,
finding the bottleneck, adding a unit there and repeating. Using only mean process times sizes the
facility too small: the stochastic model needs more equipment than the deterministic one for the
same target.

**For StochCraft.** A model of this shape is the acceptance test for M2: shared pools (M2.5),
calendars (M2.6), a *probability of meeting a target* metric and an automatic capacity-sizing
search (M2.8). StochCraft writes its own model of this kind with its own numbers.

> Z. Matheson, T. Cook, G. Sandler, J. Anderson, *Discrete Event Simulation as a Tool for Cost
> Estimating*, NNSA Office of Programming, Analysis and Evaluation, presentation, 27 April 2022.

### Ground operations at Kennedy Space Center (2010)

Three methods from KSC's DES centre:

- **Expert elicitation (the Delphi method)** where no data exist: each expert gives a minimum, most
  likely and 95th-percentile duration per task, anonymously, over two or three rounds; the median
  triad is fitted with a triangular, Weibull or lognormal distribution. At least three experts per
  task.
- **Common-cause and special-cause variation:** ordinary variation lives in the task-time
  distribution; rare contingencies are separate branches with their own probability.
- **Roll-up:** results of a detailed model become input distributions of a coarser one.

Requirements are checked as hypotheses with stated producer's and consumer's risks.

**For StochCraft.** Distributions fitted from three-point estimates, written as standard BPSim
distributions with the triad kept as the parameter's pedigree (M2.8); probability branches
(M2.3); results turned into an empirical (BPSim user) distribution for another model (M2.8).

> L. Trocine, N. H. Cummings, A. M. Bazzana, N. Rychlik, K. L. LeCroy, G. R. Cates,
> M. Mollaghasemi, *Statistical and Probabilistic Extensions to Ground Operations' Discrete Event
> Simulation Modeling*, AIAA SpaceOps 2010 (NASA Technical Reports Server 20100037919).

### Launch and assembly campaigns (Aerospace Corporation, 2023)

A summary of DES campaign analyses for lunar, asteroid and Mars missions (2006–2016): launch
order and separation, loiter time in orbit, the probability of assembling a vehicle before its
departure window. Delays come from discrete empirical distributions built from Space Shuttle
history. Results are probabilities and cumulative distributions over time with confidence bands,
not steady-state averages. Guidelines: model at the level of detail the data support and the
question needs, and finish in time to be useful.

**For StochCraft.** *Terminating* experiments (run until an event or until the model empties),
time-to-event metrics, and results as percentiles and cumulative distributions with confidence
bands, alongside mean ± half-width (M2.8).

> G. Cates, *Past Discrete Event Simulations: Summary of past lunar, asteroid, and Mars mission
> campaign analyses*, The Aerospace Corporation, OTR 2023-00381, January 2023.

### Next-generation DES for human-centric manufacturing (2022)

A research agenda in six gaps: extended reality, hybrid DES and agent simulation, machines and
simulators that talk to each other, AI that explains automated decisions in plain language, DES
as the interface to the shop floor, and game and physics engines. Digital twins are described at
three levels (supervisory, interactive, predictive); DES provides the predictive "what-if" level.
An audit trail should justify automated decisions.

**For StochCraft.** The engine's journal is an audit trail; agents over MCP (M3.6) can explain
results ("the oven is the bottleneck at 94% utilisation"); the renderer can use a game engine
(Bevy). For digital twins, a run must be able to start from an observed state (After M3).

> C. J. Turner, W. Garn, *Next generation DES simulation: A research agenda for human centric
> manufacturing systems*, Journal of Industrial Information Integration 28 (2022) 100354,
> https://doi.org/10.1016/j.jii.2022.100354.

### Parallel discrete-event simulation (WarpIV, 2021)

A NASA SBIR report on the WarpIV kernel: optimistic (Time Warp) synchronisation with rollback, to
run one large simulation across many cores, developed on a laptop and moved to HPC unchanged.

**For StochCraft.** Not planned. Independent replications in parallel already use every core for
the models StochCraft targets; parallelising a single run matters only for very large models.

> J. Steinman, *Technologies for Large-Scale Numerical Simulation*, Phase I final report, WarpIV
> Technologies, NASA contract NSSC20C057080, February 2021.

## Digital twins and Industry 5.0

Six reviews agree that simulation is the predictive, what-if part of a digital twin, fed by data
from its physical counterpart. Industry 5.0 adds three aims to Industry 4.0's: human-centric,
sustainable and resilient production.

**For StochCraft (After M3).** Starting a run from a factory's current state; calibrating input
distributions from event logs; workers as resources with skills and shifts; energy and CO₂ per
part and per element (BPSim cost parameters plus a vendor extension); disruption scenarios
(breakdowns arrive in M2.6). Open standard files (BPMN, BPSim) are what let a twin exchange
models with other tools.

> J. Barata, I. Kayser, *Industry 5.0 – Past, Present, and Near Future*, Procedia Computer Science
> 219 (2023) 778–788.
>
> J. Barata, I. Kayser, *How will the digital twin shape the future of Industry 5.0?*,
> Technovation 134 (2024) 103025, https://doi.org/10.1016/j.technovation.2024.103025.
>
> N. Anwer, R. Stark, F. Tao, J. A. Erkoyuncu, *Developing and leveraging digital twins in
> engineering design*, CIRP Annals 74 (2025) 843–868, https://doi.org/10.1016/j.cirp.2025.05.002.
>
> O. Bongomin et al., *Digital twin technology advancing industry 4.0 and industry 5.0 across
> sectors*, Results in Engineering 26 (2025) 105583, https://doi.org/10.1016/j.rineng.2025.105583.
>
> J. Monteiro, J. Barata, *Digital twin-enabled regional food supply chain: A review and research
> agenda*, Journal of Industrial Information Integration 45 (2025) 100851,
> https://doi.org/10.1016/j.jii.2025.100851.
>
> C. Kang, M. Walker, J.-H. Bartels, G. Marzahn, S. Marx, *Digital twin technologies for bridge
> lifecycle management: Literature insights and a pilot study on the Nibelungen Bridge*, Results
> in Engineering 28 (2025) 108288, https://doi.org/10.1016/j.rineng.2025.108288.

## Application domains

Material-flow and techno-economic studies of process industries: glass making and its
decarbonisation, recovering glass and metals from incinerator bottom ash, packaging waste,
CO₂ capture and transport at an ammonia plant, hydrogen in industry, and emission pathways for
energy-intensive industries. What a simulator would model in them is mostly *bulk* flow:
separators that split a stream by fractions, sorters with a throughput limit, storage tanks,
batch treatment, long furnace campaigns.

**For StochCraft.** A bulk-flow library (rates, splits, tanks) belongs with stock-and-flow in the
`sd` crate (After M3) rather than in M2's discrete blocks. These studies are sources of example
models.

> M. Zier, N. Pflugradt, P. Stenzel, L. Kotzur, D. Stolten, *Industrial decarbonization pathways:
> The example of the German glass industry*, Energy Conversion and Management: X 17 (2023) 100336,
> https://doi.org/10.1016/j.ecmx.2022.100336.
>
> S. A. Delbari, L. A. Hof, *Glass waste circular economy – Advancing to high-value glass sheets
> recovery using industry 4.0 and 5.0 technologies*, Journal of Cleaner Production 462 (2024)
> 142629, https://doi.org/10.1016/j.jclepro.2024.142629.
>
> J. Mühl, S. Skutan, G. Stockinger, D. Blasenbauer, J. Lederer, *Glass recovery and production of
> manufactured aggregate from MSWI bottom ashes from fluidized bed and grate incineration by means
> of enhanced treatment*, Waste Management 168 (2023) 321–333,
> https://doi.org/10.1016/j.wasman.2023.05.048.
>
> J. Mühl, S. Hofer, D. Blasenbauer, J. Lederer, *Recovery of aluminum, magnetic ferrous metals and
> glass through enhanced industrial-scale treatment of different MSWI bottom ashes*, Waste
> Management 190 (2024) 557–568, https://doi.org/10.1016/j.wasman.2024.10.025.
>
> A.-M. Lipp, J. Lederer, *The circular economy of packaging waste in Austria: An evaluation based
> on statistical entropy and material flow analysis*, Resources, Conservation & Recycling 217
> (2025) 108193, https://doi.org/10.1016/j.resconrec.2025.108193.
>
> H. A. Haugen, N. H. Eldrup, A. M. Fatnes, E. Leren, *Commercial capture and transport of CO2 from
> production of ammonia*, Energy Procedia 114 (2017) 6133–6140.
>
> K. Röper, N. Kunz, L. Gast, *Renewable hydrogen in industrial production: A bibliometric analysis
> of current and future applications*, International Journal of Hydrogen Energy 98 (2025)
> 687–696, https://doi.org/10.1016/j.ijhydene.2024.12.034.
>
> P. Diesing, G. Lopez, P. Blechinger, C. Breyer, *From knowledge gaps to technological maturity:
> A comparative review of pathways to deep emission reduction for energy-intensive industries*,
> Renewable and Sustainable Energy Reviews 208 (2025) 115023,
> https://doi.org/10.1016/j.rser.2024.115023.

## Read and set aside

Outside what a process simulator does: Type IV hydrogen pressure vessels (Khan, IJHE 2026,
https://doi.org/10.1016/j.ijhydene.2026.155422), polyester textile composites (Ferreira Junior et
al., J. Cleaner Prod. 2025, https://doi.org/10.1016/j.jclepro.2025.145759), fused deposition
modelling parameters (Sukumaar et al., AIEPR 2026, https://doi.org/10.1016/j.aiepr.2025.11.006),
federated learning for industrial IoT (Al-Shamasneh et al., Results in Engineering 2025,
https://doi.org/10.1016/j.rineng.2025.106387), and gust modelling in the FUN3D CFD code (Bartels,
NASA/TM-2012-217771), whose verification against analytic solutions mirrors StochCraft's
queueing-theory tests.
