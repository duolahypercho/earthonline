# Playable vehicle physics

Local play and the network client select the sedan by default. Set `--vehicle hatchback`, `--vehicle sedan` or `--vehicle van` at launch. Add `--occupants-kg` and `--cargo-kg` to specify the total occupant and cargo mass in kilograms. For example, `--vehicle van --occupants-kg 80 --cargo-kg 600` places 80 kg at the seats and 600 kg in the cargo area. The compatibility option `--vehicle catalogue-sedan` selects the original unloaded handling model.

In a car, W applies power in the selected range, S applies the service brake in either direction, and R switches between Drive and Reverse near standstill. The standard gamepad input map assigns these to RT, LT and X. A request above 0.25 m/s is discarded. The window title shows the selected range. Each tyre supplies braking force within its available grip, including while stopped on a hill.

## What determines how the car moves

Rapier integrates the chassis, its angular momentum and gravity at 60 Hz. Gravity is 9.80665 m/s². Each tire has a contact ray at its model's wheel centre. Compression, damping and suspension travel determine the load at each contact. The tire can transmit only the longitudinal and lateral impulse available from that load and the road grip. Applying the impulse at the wheel's contact point produces pitch, roll and yaw about the loaded centre of mass. A wheel in the air supplies no tire force.

At a rounded crest the required downward acceleration is approximately speed squared divided by the radius of curvature. When gravity supplies enough acceleration, the tyres can stay in contact. When the road drops away faster, the car carries its takeoff velocity forward and falls under gravity and air resistance. Chassis contacts on landing can act before the tyres reach the ground. Shared triangle edges in roads and decks use their adjacent surface normals when the chassis meets the paving in a dip.

The front wheels steer through different angles around a common turning centre. An analog steering axis requests the rack angle directly. Keyboard input ramps toward the requested steer and reduces sensitivity at higher speeds. Engine torque reaches the driven wheels through the selected gear and final drive. The sedan and hatchback use front-wheel drive and the van uses rear-wheel drive. Service brakes act on all four wheels. The parking brake acts on the rear axle. The model uses wind-relative aerodynamic drag, rolling resistance, wet grip and local standing-water depth.

Occupants and cargo change the chassis mass, centre of mass, principal inertias and static axle loads. The springs retain their curb-mass stiffness as the load changes. A natural consequence is a lower ride height at the more heavily loaded axle. Under gravity two airborne cars of different masses have essentially the same vertical acceleration.

## Mechanical references and estimates

| Playable body | Reference configuration | Curb mass | Rated engine output | Peak torque |
| --- | --- | ---: | ---: | ---: |
| Hatchback | Fiesta 1.6 L, five forward gears | 2,536 lb | 120 hp at 6,350 rpm | 112 lb-ft at 5,000 rpm |
| Sedan | Fusion 2.5 L, six forward gears | 3,472 lb | 175 hp at 6,000 rpm | 175 lb-ft at 4,500 rpm |
| Delivery van | Transit cargo, regular wheelbase, medium roof, 3.5 L port/direct-injection engine, rear drive | 5,115 lb | 275 hp at 6,500 rpm | 262 lb-ft at 3,750 rpm |

The passenger-car curb masses, power, torque and driven axles come from the manufacturer's 2018–19 US product guide. The van values and ten forward gear ratios come from the 2021 US technical specifications. [Passenger car product guide](https://xr793.com/wp-content/uploads/2025/01/2019-Ford-Vehicle-Guide.pdf) · [Van technical specifications](https://media.ford.com/content/dam/fordmedia/North%20America/US/product/2021/transit/21Transit_TechSpecs.pdf).

The hatchback's five ratios and 4.07 final drive agree with the 2018 owner's manual to the precision used in the game. The sedan uses the published six-speed ratios for an earlier 2.5 L model. The van's 3.73 final drive is listed for the regular-wheelbase, medium-roof, rear-drive cargo configuration in the 2021 towing guide. [Five-speed manual](https://www.fordservicecontent.com/Ford_Content/vdirsnet/OwnerManual/Home/Content?ProcUid=G1606816&Uid=G1606814&buildtype=web&countryCode=USA&div=f&languageCode=en&moidRef=G1606812&userMarket=CAN&vFilteringEnabled=False&variantid=6041) · [Six-speed specifications](https://media.ford.com/content/dam/fordmedia/North%20America/US/product/2016/2016-Ford-Fusion-Tech-Specs-FINAL.pdf) · [2021 cargo van axle ratios](https://www.ford.com/cmslibs/content/dam/brand_ford/en_us/brand/resources/general/pdf/guides/2021_Ford_RVandTrailerTowingGuide_r6_Oct20.pdf).

Wheel radii, mounting positions and wheel tracks follow the authored game bodies. Front axle weight shares, centres of mass, load locations, diagonal inertias, suspension rates, tire coefficients, aerodynamic area, drivetrain efficiency and torque between the published points are estimates. Payload limits of 400 kg for the hatchback, 450 kg for the sedan and 1,500 kg for the van are simulation limits. Gear selection has hysteresis. A shift interrupts wheel torque and then re-engages the clutch. Estimated shift durations are 40 ticks for the five-speed manual and 12 ticks for the automatic transmissions. A saved player checkpoint includes the selected direction, forward gear and remaining shift time.

The straight-line hatchback test starts on level, dry pavement with 75 kg in the cabin. It records elapsed time from rest and from the first foot of travel, then brakes from 70 mph. The comparison car was a US-market 120 hp five-speed hatchback weighing 2,566 lb at the curb. It reached 60 mph in 10.1 seconds, stopped from 70 mph in 181 ft (55.17 m), and achieved 0.81 g on a 300 ft diameter skidpad. The integration test allows a 10% cross-model-year difference in acceleration time and stopping distance. The publication describes a change to its rollout procedure in 2019. The hatchback uses a calibrated dry grip coefficient of 0.885. The circular test drives the hatchback around a 45.72 m radius for 20 seconds at each requested speed, in both directions. In the last 16 seconds of each run it measures lateral acceleration perpendicular to the actual velocity, wheel contact and distance from the circle. The highest sustained dry acceleration within 2.5 m of the line was 0.798 g. At higher demand the car ran wide and reached 0.825 g. With a saturated road film, measured acceleration peaked at 0.717 g. The wet result describes the simulation. [Instrumented US hatchback test](https://www.caranddriver.com/reviews/comparison-test/a15129169/2011-ford-fiesta-vs-2010-honda-fit-2011-mazda-2-comparison-test/) · [Road-test procedure](https://www.caranddriver.com/features/a32018270/how-we-test-cars/).

The sedan follows the same dry-pavement procedure as the hatchback. The comparison for acceleration is the 8.9 seconds measured for an earlier 175 hp, 2.5 L four-cylinder sedan. In the game the sedan reached 60 mph in 8.23 seconds from rest and stopped from 70 mph in 51.62 m. The braking reference is 180 ft (54.86 m) from 70 mph, measured on a lighter 3,340 lb manual version. The same manual sedan reached 0.82 g on a 300 ft diameter skidpad. The simulated sedan sustained 0.845 g clockwise and counterclockwise within 2.5 m of that circle. These comparisons use a 10% cross-model-year envelope. A test of the exact sedan configuration and an instrumented test of the naturally aspirated van are still needed. [Four-cylinder acceleration test](https://www.edmunds.com/ford/fusion/2012/review/) · [Sedan braking and skidpad test](https://www.caranddriver.com/reviews/comparison-test/a15147842/2010-ford-fusion-vs-mazda-6-honda-accord-comparison-test/).

The circle fixtures use curb mass plus the stated cargo. The delivery van exercises a moderate 0.40 g bend in both directions on the same circle. Empty and with 600 kg of cargo, the simulated van achieved 0.409–0.410 g and stayed within 0.20 m of the line on dry pavement and with a saturated road film. This measures tracking below the grip limit. An instrumented test of the reference van is still needed to validate its maximum cornering acceleration.

## Floors, people and other vehicles

The player's swept footprint makes late collision tiles resident before a fixed step enters them. A parked car waits at its saved pose while its support is outside the streaming radius. Surface queries use the actor's approach height on stacked roads and floors. Vertical ground probes subtract each candidate collider’s position before converting to single precision. This preserves centimetre-scale height differences where thin triangles meet at steep angles far from the map origin. The house at 21 Beaver Street derives walking contacts from the finished top faces of the three storeys of floorboards as well as the underlying structural slabs. The stairwell openings follow the authored geometry. A rider on a moving moored hull inherits the velocity at the point of contact, including when jumping off. Local pedestrians yield to opposing walkers and an on-foot player acts as an obstacle at street level.

At US 101 and Mission Street, two sidewalk corners and the connecting street’s kerb extended into the mapped exit ramp. A front corner of the sedan could strike the kerb almost on the ramp centreline. At Market Street, a corner extended across a lane of Octavia Boulevard just beyond the ramp. At these two interchanges the street builder uses the source way IDs to cut the overlapping sidewalk back to the surveyed carriageway edge. This distinction matters because a mapped centreline can also legitimately cross a pedestrian ramp. Both the junction corners and the sidewalk along the connecting street stop at the mapped through lane. The visible sidewalk, the pedestrian floor and the vehicle barrier use the same triangles. The regression drives both sedans, the hatchback and the van through both locations. Each run keeps wheel contact through the junction, including the van carrying 600 kg. It also checks that the remaining Mission Street sidewalk supports a pedestrian.

Loading collision tiles makes fresh surfaces available for queries between simulation ticks. Each fixed tick advances moving bodies once, including when the player is riding a lift. A resident car with its parking brake on receives the same wind and road wetness as the car being driven.

At the Ocean Beach Esplanade and Sunset Dunes, the concrete panels, landings and stair treads supply their drawn triangles to the collision world. Parapets and risers block movement. Pedestrians use the panel tops with either crowd setting. The character can stand on the lip of a tread while stepping up to the next one. The same geometry is present in dedicated servers and prediction worlds. Some lower Sunset Dunes treads intersect the mapped terrain, so the ground surface covers them in play.

Local traffic near a possible impact uses dynamic chassis. Tires and brakes follow the traffic route after a collision. Distant traffic follows its simulated route. Moving, corridor and parked traffic select their mechanical reference from the body drawn. Kerbside cars use the same measured wheel positions and chassis bounds as moving cars when their meshes are available. A parking brake resists a shove, and a displaced parked car resumes its physical pose when the player returns to its block. A partially installed fleet uses the mass of its visible fallback. In multiplayer the server resolves normal and friction impulses between players' cars and sends the resulting states to the predicting clients. These contacts use the existing simulation clock.

The model-footprint preview gives the authoritative traffic contact body the
same cooked chassis bounds and wheel positions as the local game. It samples
the road at the loaded model's wheelbase and selects the mechanical reference
using the model drawn for that vehicle ID. See the scale report and preview
controls in [Traffic Lab](TRAFFIC-LAB.md).

### Driveways on hills

Where a service drive meets a street between junctions, each wheel can reach the sidewalk at a different height. The street mesh fits the driveway across its width and grades the approach into the receiving pavement. Up to that seam the driveway supplies the floor. The sidewalk slab and its retaining wall end at the sides of the entrance. The depressed kerb keeps the local 25 mm lip and the apron rises to the existing sidewalk. The drawn triangles supply collision for pedestrians and vehicles. [San Francisco driveway detail](https://sfpublicworks.org/sites/default/files/DPW%20updated%20standard%20driveway%20plan.pdf)

Saint Charles Avenue and the asphalt service drive approaching it use classified pavement returns. At the south end of the avenue the street meets the bridge deck above the terrain grid. The regression drives a sedan with an 80 kg occupant and a van with the same occupant and 600 kg of cargo out of the service drive, over the sidewalk and into the avenue. It checks contact at all four wheels and the change in speed on every simulation step. A second regression walks three lines along the drive and three across the sidewalk, each in both directions. [Survey inputs and elevations](../native/eo-data/data/bridge-roads.NOTICE.txt)

### Divided streets and junctions

A street can arrive as several map records even on an uninterrupted bend. Where exactly two compatible ends meet on the same mapped way, their paved cross-sections share the bend normal. Intermediate rows follow the two edges up to the first unaffected row. Each record retains its traffic identity. At the join the paved faces extend by one coordinate precision unit so later triangle clipping cannot reopen the edge. The full-map regression samples the outside wheel track on bends, including the Mariposa entrance, Masonic Avenue, Indiana Street and Terry A. Francois Boulevard. [Bend construction](../native/eo-street/src/continuous.rs) · [Street and collision checks](../native/eo-player/tests/collision/continuous_streets.rs)

At Market Street and Castro Street, a ramp boundary sits 9 mm from a surveyed bend. Each additional cross-section follows the two existing pavement edges through the bend. This keeps the inner wheel track from folding back over itself. Where two mapped directions of a boulevard overlap within one kerb height, the upper asphalt carries the wheel and the priority road owns the meeting line.

Farther along Market Street, two carriageways meet where their relative height changes. Rounding their edges separately once left a 0.09 mm gap in the drawn asphalt. The cut now accounts for both grades and the precision of the mesh coordinates. The street regression probes the seam and its neighbouring positions at mesh precision. The player regression casts onto each asphalt layer there and compares the hit with the exact footprint of the drawn triangle. [Street surface test](../native/eo-street/tests/surface.rs)

At San Jose Avenue and Broad Street, the opposite corner's sidewalk extended into the through lane. The street builder now trims the sidewalk, its vertical faces and the earth shoulder to the mapped carriageway. Inside the junction, only the part of the lane covered by the drawn junction asphalt can open the corner. The regression drives a sedan and a loaded delivery van over both the junction and the adjoining lane. [Vehicle contact tests](../native/eo-player/tests/collision.rs)

The cut follows the elevation of the road so the lower pavement and retaining face beneath Hyde Street survive. At Lake Merced Boulevard and Middlefield Drive, the warning panels follow the surviving curb ramps. The placement check samples the centre and corners of each panel and considers every floor at those coordinates, including sidewalks beneath a deck. [Street and bridge regressions](../native/eo-structures/tests/joins.rs) · [Warning-panel regression](../native/eo-props/src/pads.rs)

Mariposa Street crosses the Caltrain tracks east of Pennsylvania Avenue and meets the southbound entrance beneath I-280. At the fork, the measured street and entrance share a pavement elevation of 12.97 m NAVD88. The lower deck behind the southwest curb also carries pedestrians. Its mesh uses complete survey cells and meets the existing sidewalk at its edge. Covered parts of the road profiles interpolate between exposed returns. The rail bed and freeway retain their own elevations. [Caltrain crossing inventory](https://www.caltrain.com/media/36811) · [Survey stations, method and source hashes](../native/eo-data/data/bridge-roads.NOTICE.txt)

The on-foot regression starts on the lower sidewalk near game X 2872.75, Z 1938.25 at the default map scale. It crosses the curb return and the apron in both directions. The vehicle regression drives the full rail crossing and the ascending entrance with a sedan and a delivery van carrying 600 kg of cargo. Beneath the freeway, the deck mesh keeps the solved crossing open where the terrain rises beside the ramp. The tests check all four wheel contacts, speed loss, the two stacked road surfaces, headroom above the van’s lane and the space between the apron and rail bed. [Mariposa controller regressions](../native/eo-player/tests/collision/mariposa.rs)

### Traffic curb weights

These are reference configurations for the fictional fleet. The suspension and rigid body use the same curb weight and axle balance. The longitudinal centre is located between the actual wheel positions. An axle carrying 32% of the weight at rest needs to carry that fraction even when the wheelbase or its midpoint differs from the reference car.

| Drawn body | Reference configuration | Curb mass used |
| --- | --- | ---: |
| Compact hatch, hatch sport and cross trims | Playable hatch | 2,536 lb |
| Sedan, sport and luxury trims | Playable sedan | 3,472 lb |
| Delivery van | Playable van | 5,115 lb |
| Wagon | Lower end of the published 2019 V60 range | 3,770 lb |
| Coupe | 2022 BRZ Premium manual, 53% on the front axle | 2,815 lb |
| Mid-engine coupe | 2020 718 Cayman GT4 manual | 3,199 lb |
| Compact SUV | Escape 2.5 L, front-wheel drive | 3,542 lb |
| Full-size SUV | Expedition MAX, rear-wheel drive | 5,534 lb |
| Compact minivan | 2015 Mazda5 automatic | 3,457 lb |
| Pickup | 2021 F-150 crew cab, 3.3 L, 4×2. Front 2,588 lb and rear 1,877 lb | 4,465 lb |
| Taxi and livery variants | Fusion Hybrid | 3,668 lb |
| Patrol sedan and SUV | 2018 Interceptor sedan EcoBoost AWD and 3.7 L utility AWD | 4,371 and 4,672 lb |
| Box truck | Empty 20 ft moving truck with box installed | 8,800 lb |
| City bus | 40 ft XD40 check-in scale measurement. Front 8,750 lb and rear 18,980 lb | 27,730 lb |

The passenger-car and utility tables specify base curb weight. The manufacturer's pickup body-builder table gives the base weight on each axle. The passenger-car references describe the same body class and general size as the game vehicles. [Ford passenger and utility tables](https://media.ford.com/content/dam/fordmedia/North%20America/US/product/2019/2019-product-guide-LR.pdf) · [Ford pickup layout book, crew cab table](https://madocumentupload.marketingassociates.com/api/Document/GetFile?v1=5601895&v2=102620091105&v3=60&v4=0bac34db9c6872211ac289bdc36c935eee0738c1766b95df28b2e64a&v5=False) · [Michigan State Police 2018 evaluation, model specifications](https://www.michigan.gov/-/media/Project/Websites/msp/training/2018MYPoliceVehicleEvaluationTestBookFINAL.pdf?rev=12c87daccf624fdd82d97b9e8bd52622).

The coupe's weight and axle balance are published together. The wagon uses the low end of the manufacturer's curb range. The mid-engine coupe's 44% front share is an estimate. [Subaru BRZ brochure](https://www.subaru.com/content/dam/subaru/downloads/pdf/brochures/2022/brz/2022_BRZ_Brochure_Final_02.pdf) · [Porsche technical specifications](https://newsroom.porsche.com/dam/jcr%3Aa6e80e45-d9d7-4cb8-a43d-16a6da449e0b/2020_718_Cayman_GT4_and_718_Spyder_tech_specs.pdf) · [Volvo V60 weights](https://www.volvocars.com/en-ca/support/car/v60/2019/article/b0804d54c7fc096bc0a81f6f065ad63e/0362eef4c7fc436fc0a81f6f7c27a289/cb9a38ec3e00fbacc0a8015143718779/) · [Mazda5 specifications](https://news.mazdausa.com/download/2015-Mazda5-Specifications.pdf).

The box truck uses the operator's published empty weight of a complete vehicle. The bus uses the laboratory's two axle readings at curb weight, before passengers boarded. [Moving-truck dimensions and weights](https://www.uhaul.com/Truck-Rentals/20ft-Moving-Truck/) · [Bus testing centre report, check-in p. 5](https://www.pa.gov/content/dam/copapwp-pagov/en/penndot/documents/programs-and-doing-business/transit/procurement/documents/new-flyer-of-america-model-xd40.pdf).

Traffic engines and gearboxes still use representative torque curves and ratios. All-wheel-drive traffic splits torque equally between axles. That split, unmeasured axle shares, centres of gravity and the rectangular-body inertia estimates remain approximations. Cosmetic trims retain the same mechanical configuration.

Where the local ground provides a mapped water column, four immersion samples apply buoyancy and drag and a submerged intake interrupts engine torque. The flooding approximation loses air over 3,600 immersed ticks. Effective fully submerged displacement decreases from 1.85 to 0.42 times curb mass in water-mass equivalents. These values are estimates. The dedicated server shares the Municipal Pier and floating-dock floors. Its ground provider lacks the local marine sampler.

### Bridge roadways

The Golden Gate Bridge and both levels of the Bay Bridge west span use the asphalt panel corners and panel diagonal from the drawn bridge for tire and foot contact. The Golden Gate median and traffic rail and the upper Bay Bridge traffic rails also have physical faces. The edge of the lower Bay roadway ends where its asphalt ends. Default play loads these surfaces with the landmarks. With `--structures`, the tile containing each panel streams it, and the approach pavement fills the angled joint between the surveyed carriageway and the bridge. Local play, prediction and the dedicated server use the same geometry.

At the Golden Gate abutments the median follows the curve and grade at both ends of each panel. The short south approach blends the street cross-section into the deck over its available length. Its concrete underside and end wall follow the low point of the pavement so the chassis can clear the joint. The street mesh trims curb and sidewalk pieces where they project into the connecting lane at road height. When a tilted suspension ray grazes a vertical face of a kerb, end wall or median, it continues to the supporting surface. The chassis still meets the wall, and the raised top still carries the wheel.

The bridge checks probe near the corners and at the centre of every drawn road triangle. The driving checks follow both Golden Gate carriageways and both Bay levels with a sedan or loaded van. They record the number of tires touching the pavement, the largest change in forward speed over one step and the distance from the vehicle datum to the drawn road. Additional drives enter the span from the surveyed north and south approaches and steer the sedan into the median at 3 m/s.

### Shared driving weather

In online play the server sends the road weather mode, schedule seed and the mapping from input sequence to simulation time at join, resume and correction. The same controller consumes rain rate, wetness, standing water, wind and gusts in local play, on the server and during prediction. Scheduled weather uses the whole world second, including during a rewind. The server changes the weather together with an authoritative player state, so a client can replay its outstanding inputs in the new conditions. A two-client storm test drives a van carrying 600 kg into a stationary hatchback and checks that both clients retain their load and weather through contact correction and input replay. A client that pauses long enough to drift from the weather schedule receives a fresh time reference.

Use `--weather storm` for a steady storm or `--weather schedule --weather-seed 123 --world-time 21000` for a repeatable scheduled front. The host's `--no-weather-fx` disables weather forces for all participants. A remote client's own presentation switches leave the host's road conditions authoritative. Protocol version 11 includes the shared weather, fixed-step loading, floor selection during vehicle transfers, ceiling contact during jumps and movement inherited from a supporting vehicle. The server and clients must use compatible builds.

### Parking and walking between floors

A person standing on a moving roof or deck follows the point under their feet as the body turns, rises and travels. Walking speed is measured relative to that point. A jump or step over the edge carries the contact velocity into the air, where gravity takes over. After the rigid bodies take their shared physics step, the standing capsule follows the actual roof displacement and sweeps against nearby walls. This includes motion caused by tire forces and suspension travel. Before a roof carries a rider across a streaming boundary, the engine loads collision for the path of their feet. A departing rider also loads the path covered by their inherited velocity. The moving-roof tests cover a simulated vehicle, both forms of scripted motion, departure over an edge, an obstruction at head height and four-second powered turns with the playable sedan and loaded delivery van. The standing rider is a kinematic character. Loading the suspension with an external rider’s weight and modelling shoe slip under abrupt braking remain open. [Moving support regressions](../native/eo-player/tests/stacked_floors.rs)

When the player parks a car nearby, the engine samples the intervening grade from their feet to the parking spot. The driver can enter when there is shoulder clearance between the walker and the cabin. For an exit, the controller searches within stepping distance of the tilted door sill and checks the standing capsule on the surface it finds. If a wall, person or drop blocks the driver's door, the instant exit tries the opposite side and then behind the car. During a jump, contact with the underside of a floor removes the velocity into it. Gravity acts again on the next tick.

Map arrivals use the surveyed carriageway when a graded approach stands above the terrain search range. At Presidio Parkway this puts the car on the pavement 5.14 m above the underlying ground. Open tunnel approaches retain a ground collider shaped by the same elevation-grid cuts as the renderer. At the portal itself, ground collision ends at the canopy boundary. The tunnel-ground audit compares drawn terrain triangle centres with collision in the Presidio approaches. Set `EO_AUDIT_ALL_TUNNEL_GROUND=1` to check every open approach in the loaded city.

The controlled parking fixture includes two floors 2.6 m apart, an inclined deck over a 36% hill, a raised paving surface and a low ceiling. The dedicated-server test walks into a covered parking space, enters, exits and jumps beneath its roof with identical input on both peers. The local furnished-house check drops a standing body through the upper stairwell and onto the adjacent floor at 21 Beaver Street.

At 21 Beaver Street, the staircase uses the installed oak treads, their rounded noses and the fitted runner for collision. Saved locations preserve the dining-room and front-stoop layouts that supported the player. The observation-room test follows the drawn floor from lift stops and viewing positions to the glazing at Coit Tower, Salesforce Tower and the Transamerica Pyramid.

### Contact with a pedestrian

With `--crowd`, regional pedestrians yield to people and traffic whose bodies overlap their height range. Vehicle clearance follows the chassis dimensions, position, pitch and roll, including when local physics moves a car away from its lane. Each sideways correction samples the path for support. At the edge of a raised walkway, the walker with room can take the remaining step. A walker inside a vehicle footprint tries the nearest reachable side. The same rules apply with `--people-space` enabled.

The Broadway Tunnel regression places a pedestrian on the street directly over a traffic lane 22.31 m below. It also moves the car onto the surface to check that the pedestrian yields there. A separate 4 m platform fixture exercises two pedestrians on different floors, two on the same floor at the edge and a car on its wheels, on its side and airborne. Additional fixtures compare an 18 cm step, a sudden 42 cm drop and a continuous incline.

The bridge and neighbourhood crowds share vehicle and pedestrian contacts. A vehicle displaced by physics occupies its current floor. When two people leave the same side of a chassis, each selects a supported spot with room for their body. A second car can block an exit on that level. At the Golden Gate Bridge, both walking routes turn on the tower plates. The feet follow the east sidewalk’s floor triangles through the detour and over the span. A person beneath the bridge keeps the lower floor.

The east sidewalk regression checks both walking lines and their intervening segments at 8,530 points, with both versions of the bridge railing. It also advances 900 fixed steps at noon with each crowd setting. In the reference run each setting produced 276,133 pedestrian positions, all within 0.01 mm vertically of the sidewalk. The measurement uses the character’s foot position. The gait review checks each shoe during animation.

Local play with `--pedestrian-physics` gives a supported pedestrian near a car a dynamic capsule. Its nominal mass is 75 kg, scaled with body volume. The chassis and capsule exchange impulses. After a strong impact the person can tumble, regain balance on a supporting surface and walk toward the route again. The renderer takes the pose and gait from the physical person. A regional or corridor identity follows the person when the crowd changes display order. A stacked floor is checked at the person's elevation before handoff. The default retains the previous walker presentation while the candidate awaits review.

The original Golden Gate railing and the access railing both stand beside the east sidewalk. Local players, the server and predicting clients use the bridge plates with either railing. The base multiplayer city also keeps its piers and docks when tower access is off. At the Municipal Pier regression point, the deck is at 2.897 m and the ground beneath it is at about 1.31 m.

A physical walker aims for the supported stance chosen by the crowd. The motor preserves travel along a car and removes travel into its side. If a car stands between a displaced walker and the route after an impact, the walker steers around its footprint. The motor checks the surface under the leading foot and at braking distance using the contact slope. Actual loss of support and vehicle impacts still use gravity and rigid-body contacts.

The deck-gap fixture has two platforms four metres above the ground and two metres apart. The walker stops 23 cm before the edge and can walk back. Connected surfaces in the same fixture include a 16% grade and a 12 cm step, each traversed in both directions. The stopped-sedan fixture holds the walker beside the vehicle for four seconds, then walks them alongside it and measures the return to the open route.

## Reproduce a contact issue

In local play, press **F8** at the problem spot. The play window shows the nearest named street in its title. The terminal also prints the world coordinates and file path. The JSON in `native/bench-out/` (or the selected `--out` directory) contains the player checkpoint, wheel contacts, current load and collision tile. Reopening a new checkpoint restores the saved street geometry, including the pavement repair switch, and the bridge, tunnel, harbor, marine and main house settings before building the collision world. New F8 checkpoints also restore the live weather mode, seed, world time and weather effects setting, so subsequent steps continue the same road conditions. Older checkpoints use the launch settings for fields they lack. The checkpoint restores the player and their cars. Traffic, boats and doors have their normal scene-start state.

From `native/`, use the following commands for a live checkpoint or a step-for-step scripted route. Route traces compare the quantized body and contact state after every fixed step. A mismatch reports its frame and both contact samples. New recordings also check the pavement repair switch before the first replay step. Older recordings use the street settings they contain.

```sh
./target/release/eo-app --play --physics-location bench-out/physics-location-TIMESTAMP.json
./target/release/eo-app --play --bench --no-render --segments drive_player --vehicle van --cargo-kg 600 --record-physics --out bench-out/van
./target/release/eo-app --play --bench --no-render --replay-physics bench-out/van/drive_player.trace.json --out bench-out/van-replay
cargo test --release -p eo-player --test fleet_dynamics --test collision --test harbour
cargo test --release -p eo-player --test stacked_floors --test tunnel_ground -- --nocapture
cargo test --release -p eo-player --test crest -- --nocapture
cargo test --release -p eo-app --bin eo-app bridge_road_tests -- --nocapture
cargo test --release -p eo-structures --test joins full_city_highways_and_bridge_wheel_tracks_are_continuous -- --exact --nocapture
cargo test --release -p eo-server --test determinism bridge_traffic_lanes_carry_the_same_loaded_van_on_server_and_client -- --exact --nocapture
cargo test --release -p eo-player --test collision sidewalk_corners_leave_the_us_101_exit_and_octavia_carriageways_drivable -- --exact --nocapture
cargo test --release -p eo-player --test collision saint_charles_driveway -- --nocapture
cargo test --release -p eo-player --test collision mariposa -- --nocapture
EO_AUDIT_ALL_STREET_TILES=dense cargo test --release -p eo-player --test collision continuous_streets -- --nocapture
cargo test --release -p eo-street --lib fitted_driveway_mouths -- --nocapture
EO_AUDIT_ALL_STREET_TILES=1 cargo test --release -p eo-player --test collision rendered_walkable_street_layers_have_physics_support -- --nocapture
EO_AUDIT_ALL_STREET_TILES=dense cargo test --release -p eo-player --test collision rendered_walkable_street_layers_have_physics_support -- --nocapture
cargo test --release -p eo-player --lib traffic_contact::tests -- --nocapture
cargo test --release -p eo-app --bin eo-app the_drawn_vehicle_selects_its_mass -- --nocapture
cargo test --release -p eo-app --bin eo-app rendered_floorboards_have_support -- --nocapture
cargo test --release -p eo-app --bin eo-app interior_floor_tests -- --nocapture
EO_AUDIT_ALL_TUNNEL_GROUND=1 cargo test --release -p eo-app --bin eo-app the_drawn_presidio_tunnel_approach_ground_supports_a_body -- --nocapture
cargo test --release -p eo-app --bin eo-app coastal_promenades_support_walkers_stair_traversal_and_vehicle_wheels -- --nocapture
cargo test --release -p eo-server --test determinism --test network
cargo test --release -p eo-net weather::tests -- --nocapture
cargo test --release -p eo-agents --test bores --test crowd_floors --test crowd_bodies -- --nocapture
cargo test --release -p eo-app --bin eo-app corridor_crowds_keep -- --nocapture
cargo test --release -p eo-app --bin eo-app the_drawn_golden_gate_east_sidewalk_catches -- --nocapture
cargo test --release -p eo-server --test determinism base_bridge_and_pier_floors -- --nocapture
cargo test --release -p eo-server --test network drive_range_and_hill_hold_on_filbert_street -- --nocapture
./target/release/eo-app --play --bench --segments pedestrian_crossing --frames 170 --vehicle sedan --pedestrian-physics --time-of-day 12 --weather clear --capture --out bench-out/crossing
```

The standard full-map audit samples the centre of one eligible drawn triangle per walkable layer per tile. The dense run checks every walkable triangle with at least 0.0025 m² of horizontal projected area, including faces extending beyond their owning tile. The furnished-house audit casts from the centre and near the three corners of every top-facing floorboard triangle. It casts a short ray through the drawn height using only physical street colliders. It reports the total misses and the coordinates and layer for the first twelve. The scripted driver exercises a particular city route. Inspect the frame budget separately from the physical assertions. The test fixtures exercise axle loads, braking, lateral grip, a legal kerb, free fall, parked cars outside the collision stream, and the first step onto a cold upper floor. The full city includes other routes and levels with different geometry.

## Physics acceptance plan

Use the same fixed 60 Hz inputs for a repeatable drive, a busy street and a multiplayer replay. Start with the three vehicle loads, smooth crest and dip, kerb, stacked floors and cold tile boundary. A seam must preserve forward momentum, a dropped body must land on the visible surface below it, and an airborne vehicle must follow its takeoff velocity under gravity and drag. Repeat the named city routes after those controlled cases pass. Press F8 to turn a newly reported location into a repeatable starting point.

1. Validate the sedan against the same model year and transmission, and the van against the naturally aspirated ten-speed configuration. Compare acceleration, emergency braking and steady-state cornering. Validate traffic axle loads and driven-wheel torque against instrumented runs, including loading and unloading a city bus. Repeat with documented loads and tire conditions. Use measured wet skidpad runs to check the wet-road estimates and validate the van at its cornering limit and repeat the sedan comparison with the exact reference configuration.
2. Apply the floorboard comparison used at 21 Beaver Street to additional authored interiors, restorable doors and moving supports. For each stacked floor, check walking across a seam, landing from above, walking underneath and stepping beyond the actual edge. The moving-roof regressions cover a standing rider, departure velocity and crossing an unloaded boundary while riding or airborne. Extend these cases to transfers between two independently moving supports. Keep coordinates and the first divergent step for each failure.
3. Exercise multiplayer traffic, moving supports and weather with simultaneous players. Compare predicted and authoritative contact states during impacts, reconnection and changes in weather. Confirm the dedicated shoreline and water column against local play before accepting marine parity.
4. Exercise pedestrian encounters on measured sidewalks, at corners and at crossings. Record pairs approaching head-on, overtaking, joining a queue, passing an on-foot player and standing on different elevations at the same map position. Measure foot clearance, horizontal capsule overlap and time to clear the encounter. Repeat in crowds and as characters move into and out of the nearby interaction area. Use the covered bridge and neighbourhood encounters to check approach speed, deceleration and time to clear a vehicle in a moving queue. Use the recordings to check foot sliding, joint motion during a fall and visible recovery. The current impact body is a single capsule.
5. Repeat the route recordings and the full 1080p benchmark on an idle target machine. The player step budgets are 0.5 ms median and 2 ms worst. The complete frame must meet the repository's 10 ms median, p95 and worst-frame targets on every path. The two independent review rounds in the [visual quality gate](VISUAL_QUALITY_GATE.md) are the release criterion for the AAA claim.
