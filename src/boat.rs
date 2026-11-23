use crate::mbtiles_renderer::MBTilesRenderer;
use crate::screen::Map;
 
const MINIMUM_SPEED :f64 = 1.0;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct BoatState 
    {
    pub longitude           : f64,
    pub latitude            : f64,
    pub position_valid      : bool,
    pub speed_over_ground   : f64,
    pub course_over_ground  : f64,
    }



impl BoatState
    {
     pub fn default() -> Self 
        {
        BoatState {
            longitude: 0.0,
            latitude: 0.0,
            position_valid: false,
            speed_over_ground: 0.0,
            course_over_ground: 0.0,
        }
        }
        
    pub fn show(
            &self,
			renderer    : &MBTilesRenderer,
			map         : &mut Map)
        -> Result<(), String>
        {
        if !self.position_valid {
            return Ok(());
        }
        //println!("show longitude : {} / latitude : {}",self.longitude,self.latitude);
        let pt = renderer.gps_to_point((self.longitude, self.latitude));
        if pt.x < 0
            || pt.y < 0
            || pt.x as u32 > map.screen_width
            || pt.y as u32 > map.screen_height
        {
            crate::diagnostics::record(
                "WARN",
                format!(
                    "Marqueur bateau hors écran: lon={:.6}, lat={:.6}, écran=({}, {})",
                    self.longitude, self.latitude, pt.x, pt.y
                ),
            );
        }
        map.show_boat(pt.x, pt.y)?;
        if self.speed_over_ground > MINIMUM_SPEED
            {
            map.show_cog(renderer.course_to_screen(self.course_over_ground), pt)?;
            }
        Ok(())
        }
        
    pub fn position(self) -> (f64, f64)
        {
        (self.longitude,self.latitude)
        }
    }
