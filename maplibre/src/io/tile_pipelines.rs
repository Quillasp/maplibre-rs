use std::collections::HashSet;

use geozero::GeozeroDatasource;
use prost::Message;

use crate::{
    error,
    io::{
        geometry_index::IndexProcessor,
        pipeline::{DataPipeline, PipelineContext, PipelineEnd, Processable},
        TileRequest,
    },
    tessellation::{zero_tessellator::ZeroTessellator, IndexDataType},
};

pub enum TileType {
    Vector(geozero::mvt::Tile),
    Raster(Vec<u8>),
}

impl From<TileType> for geozero::mvt::Tile {
    fn from(tile_type: TileType) -> Self {
        match tile_type {
            TileType::Vector(tile) => tile,
            TileType::Raster(_) => unimplemented!(),
        }
    }
}

#[derive(Default)]
pub struct ParseTile;

impl Processable for ParseTile {
    type Input = (TileRequest, Box<[u8]>);
    type Output = (TileRequest, geozero::mvt::Tile);

    // TODO (perf): Maybe force inline
    fn process(
        &self,
        (tile_request, data): Self::Input,
        _context: &mut PipelineContext,
    ) -> Result<Self::Output, error::Error> {
        let tile = geozero::mvt::Tile::decode(data.as_ref()).expect("failed to load tile");
        Ok((tile_request, tile))
    }
}

#[derive(Default)]
pub struct IndexLayer;

impl Processable for IndexLayer {
    type Input = (TileRequest, TileType);
    type Output = (TileRequest, geozero::mvt::Tile);

    // TODO (perf): Maybe force inline
    fn process(
        &self,
        (tile_request, tile): Self::Input,
        context: &mut PipelineContext,
    ) -> Result<Self::Output, error::Error> {
        let index = IndexProcessor::new();

        // FIXME: Handle result
        context
            .processor_mut()
            .layer_indexing_finished(&tile_request.coords, index.get_geometries())?;
        Ok((tile_request, tile.into()))
    }
}

#[derive(Default)]
pub struct TessellateLayer;

impl Processable for TessellateLayer {
    type Input = (TileRequest, geozero::mvt::Tile);
    type Output = (TileRequest, TileType);

    // TODO (perf): Maybe force inline
    fn process(
        &self,
        (tile_request, mut tile): Self::Input,
        context: &mut PipelineContext,
    ) -> Result<Self::Output, error::Error> {
        let coords = &tile_request.coords;

        for layer in &mut tile.layers {
            let cloned_layer = layer.clone();
            let layer_name: &str = &cloned_layer.name;
            if !tile_request.layers.contains(layer_name) {
                continue;
            }

            tracing::info!("layer {} at {} ready", layer_name, coords);

            let mut tessellator = ZeroTessellator::<IndexDataType>::default();
            if let Err(e) = layer.process(&mut tessellator) {
                // FIXME: Handle result
                context
                    .processor_mut()
                    .layer_unavailable(coords, layer_name)?;

                tracing::error!(
                    "layer {} at {} tesselation failed {:?}",
                    layer_name,
                    &coords,
                    e
                );
            } else {
                // FIXME: Handle result
                context.processor_mut().layer_tesselation_finished(
                    coords,
                    tessellator.buffer.into(),
                    tessellator.feature_indices,
                    cloned_layer,
                )?;
            }
        }

        Ok((tile_request, TileType::Vector(tile)))
    }
}

#[derive(Default)]
pub struct TilePipeline;

impl Processable for TilePipeline {
    type Input = (TileRequest, TileType);
    type Output = (TileRequest, TileType);

    fn process(
        &self,
        (tile_request, tile): Self::Input,
        context: &mut PipelineContext,
    ) -> Result<Self::Output, error::Error> {
        let coords = &tile_request.coords;

        if let TileType::Vector(vector_tile) = &tile {
            let available_layers: HashSet<_> = vector_tile
                .layers
                .iter()
                .map(|layer| layer.name.clone())
                .collect::<HashSet<_>>();

            for missing_layer in tile_request.layers.difference(&available_layers) {
                // FIXME: Handle result
                context
                    .processor_mut()
                    .layer_unavailable(coords, missing_layer)?;

                tracing::info!(
                    "requested layer {} at {} not found in tile",
                    missing_layer,
                    &coords
                );
            }
        }

        tracing::info!("tile tessellated at {} finished", &tile_request.coords);

        // FIXME: Handle result
        context
            .processor_mut()
            .tile_finished(&tile_request.coords)?;

        Ok((tile_request, tile))
    }
}

pub fn build_vector_tile_pipeline() -> impl Processable<Input = <ParseTile as Processable>::Input> {
    DataPipeline::new(
        ParseTile,
        DataPipeline::new(
            TessellateLayer,
            DataPipeline::new(TilePipeline, PipelineEnd::default()),
        ),
    )
}

#[derive(Default)]
pub struct RasterLayer;

impl Processable for RasterLayer {
    type Input = (TileRequest, Box<[u8]>);
    type Output = (TileRequest, TileType);

    fn process(
        &self,
        (tile_request, data): Self::Input,
        context: &mut PipelineContext,
    ) -> Result<Self::Output, error::Error> {
        let coords = &tile_request.coords;
        let data = data.to_vec();

        // FIXME: Handle result
        context.processor_mut().layer_raster_finished(
            coords,
            "raster".to_string(),
            data.clone(),
        )?;

        Ok((tile_request, TileType::Raster(data)))
    }
}

pub fn build_raster_tile_pipeline() -> impl Processable<Input = <RasterLayer as Processable>::Input>
{
    DataPipeline::new(
        RasterLayer,
        DataPipeline::new(TilePipeline, PipelineEnd::default()),
    )
}

#[cfg(test)]
mod tests {
    use super::build_vector_tile_pipeline;
    use crate::{
        coords::ZoomLevel,
        io::{
            pipeline::{PipelineContext, PipelineProcessor, Processable},
            TileRequest,
        },
    };
    pub struct DummyPipelineProcessor;

    impl PipelineProcessor for DummyPipelineProcessor {}

    #[test] // TODO: Add proper tile byte array
    #[ignore]
    fn test() {
        let mut context = PipelineContext::new(DummyPipelineProcessor);

        let pipeline = build_vector_tile_pipeline();
        let _output = pipeline.process(
            (
                TileRequest {
                    coords: (0, 0, ZoomLevel::default()).into(),
                    layers: Default::default(),
                },
                Box::new([0]),
            ),
            &mut context,
        );
    }
}
