# Part-drop lists shared by headless_build.py (LOD1/LOD2 export) and build_lod2.py.
import bpy, sedan_lib
LOD1_DROP = ('Headlamp_L_LowBeam', 'Headlamp_R_LowBeam', 'Headlamp_L_HighBeam', 'Headlamp_R_HighBeam', 'Indicator_F', 'BrakeLamp_', 'ReverseLamp_', 'FogLamp_',
             'DomeLight', 'SunVisor', 'Glovebox', 'Console', 'Gear', 'Pedal', 'FuelCap', 'FuelWell', 'Vent', 'Dials', 'CentreScreen', 'ScreenBezel', 'Binnacle',
             'Wheel_FL_Groove', 'Wheel_FR_Groove', 'Wheel_RL_Groove', 'Wheel_RR_Groove', 'Wheel_FL_Disc', 'Wheel_FR_Disc', 'Wheel_RL_Disc', 'Wheel_RR_Disc',
             'Wheel_FL_Caliper', 'Wheel_FR_Caliper', 'Wheel_RL_Caliper', 'Wheel_RR_Caliper', 'Shut', 'BumperSeam', 'DiffuserFin', 'Mirror_L_Glass', 'Mirror_R_Glass',
             'DoorPull', 'DoorInsert', 'RearShelfRib', 'SteeringColumn', 'Tunnel', 'Firewall', 'EngineBlock', 'EngineCover')
LOD2_DROP = LOD1_DROP + ('Headlamp_', 'Taillamp_L_Bar', 'Taillamp_R_Bar', 'Indicator_', 'HighMountStop', 'Seat', 'Dash', 'Floor', 'RearShelf', 'Headliner', 'Steering',
             'RearViewMirror', 'DoorCard', 'DoorArmrest', 'GrilleSlat', 'GrilleFrame', 'GrilleRib', 'Handle_', 'CharLine', 'Antenna', 'Exhaust', 'FuelDoor', 'Wiper', 'Mirror_L_Arm', 'Mirror_R_Arm',
             'Wheel_FL_Spoke', 'Wheel_FR_Spoke', 'Wheel_RL_Spoke', 'Wheel_RR_Spoke', 'Wheel_FL_Cap', 'Wheel_FR_Cap', 'Wheel_RL_Cap', 'Wheel_RR_Cap', 'Window_', 'SideFront', 'SideRear',
             'GasketLo', 'GasketHi', 'GasketA', 'GasketB', 'EngineBay', 'TrunkBay', 'Rocker', 'Underbody', 'Intake')
LOD2_KEEP = ('Headlamp_L_Housing', 'Headlamp_R_Housing', 'Headlamp_L_Cover', 'Headlamp_R_Cover', 'Headlamp_L_DRL0', 'Headlamp_R_DRL0', 'Headlamp_L_DRL3', 'Headlamp_R_DRL3', 'Headlamp_L_DRL6', 'Headlamp_R_DRL6')
def drop_parts(prefixes, keep=()):
    for ob in list(sedan_lib.root().all_objects):
        n = ob.name
        if any(n.startswith(k) for k in keep): continue
        if any(n.startswith(p) for p in prefixes): bpy.data.objects.remove(ob, do_unlink=True)
