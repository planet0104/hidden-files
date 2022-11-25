use anyhow::anyhow;
use bincode::{config, Decode, Encode};
use byte_unit::Byte;
use rfd::FileDialog;
use slint::SharedString;
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::windows::prelude::{FileExt, MetadataExt},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use crate::slint_generatedApp;

const START_BYTES: &str = "RUSTAPPEND666S";
const END_BYTES: &str = "RUSTAPPEND666E";

#[derive(Clone, Debug, Default, Encode, Decode)]
pub struct FileSpec {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub sizemb: String,
    pub extension: String,
}

impl From<&slint_generatedApp::FileSpec> for FileSpec {
    fn from(f: &slint_generatedApp::FileSpec) -> Self {
        Self {
            path: f.path.to_string(),
            name: f.name.to_string(),
            size: f.size.to_string().parse().unwrap(),
            sizemb: f.sizemb.to_string(),
            extension: f.extension.to_string(),
        }
    }
}
impl From<&FileSpec> for slint_generatedApp::FileSpec {
    fn from(f: &FileSpec) -> Self {
        Self {
            path: SharedString::from(f.path.clone()),
            name: SharedString::from(f.name.clone()),
            size: SharedString::from(format!("{}", f.size)),
            sizemb: SharedString::from(f.sizemb.clone()),
            extension: SharedString::from(f.extension.clone()),
        }
    }
}

pub fn get_file_name(file: Option<PathBuf>) -> Option<(String, String)> {
    let file = file?;
    let file_name = file.file_name()?.to_str()?.to_string();
    let file_path = file.to_str()?.to_string();
    Some((file_name, file_path))
}

/// 选择文件
pub(crate) fn pick_file(filter: Option<(&str, &[&str])>) -> Option<FileSpec> {
    let mut dlg = FileDialog::new();
    if let Some((filter_name, extensions)) = filter {
        dlg = dlg.add_filter(filter_name, extensions);
    }

    let (file_name, file_path) = get_file_name(dlg.pick_file())?;

    let extension = file_name
        .split(".")
        .last()
        .unwrap_or("")
        .to_uppercase()
        .to_string();

    if let Ok(data) = fs::metadata(&file_path) {
        let size = data.len();
        return Some(FileSpec {
            path: file_path,
            name: file_name,
            size,
            sizemb: get_size_str(size),
            extension,
        });
    }

    None
}

/// 检测源文件中是否有附加文件
pub fn check_file(src_file_spec: &FileSpec) -> anyhow::Result<Option<(FileSpec, u64, u64)>> {
    let src_file = File::open(&src_file_spec.path)?;

    //从文件结尾处查找开始字节
    let file_size = src_file_spec.size;
    let mut buf = vec![0; START_BYTES.len()];
    let start_pos = file_size - START_BYTES.len() as u64;
    let len = src_file.seek_read(&mut buf, start_pos)?;
    if len != buf.len() {
        return Ok(None);
    }
    if buf != START_BYTES.as_bytes() {
        return Ok(None);
    }
    // 字节匹配，寻找结束地址
    let mut buf = [0; 1];
    let mut data = vec![];
    let mut offset = start_pos;
    let end_bytes = END_BYTES.as_bytes();
    loop {
        if src_file.seek_read(&mut buf, offset)? == 1 {
            offset -= 1;
            data.push(buf[0]);

            if data.len() > 4096 {
                println!("数据太长，提前结束！");
                break;
            }

            //最后END_BYTES.len()个字节反转对比
            if data.len() > END_BYTES.len() {
                let mut data_end_arr = data[data.len() - END_BYTES.len()..].to_vec();
                data_end_arr.reverse();
                if data_end_arr == end_bytes {
                    let mut spec_data = data[..data.len() - END_BYTES.len()].to_vec();
                    spec_data.reverse();
                    let (f, _u): (FileSpec, usize) =
                        bincode::decode_from_slice(&spec_data, config::standard())?;

                    let end_offset = offset + 1;
                    let start_offset = end_offset - f.size;

                    return Ok(Some((f, start_offset, end_offset)));
                }
            }
        } else {
            break;
        }
    }
    Ok(None)
}

/// # 保存文件和附件
///
/// 参数:
/// * `src_file_spec`: 源文件信息
/// * `append_file_spec`: 附加文件信息
/// * `output_file_name`: 合并后保存的路径
/// * `progress_callback`: 进度回调函数
/// * `is_cancled`: 读写锁，用来检测操作是否取消
pub(crate) fn copy_file<F: Fn(i32)>(
    src_file_spec: &FileSpec,
    append_file_spec: &FileSpec,
    output_file_name: &str,
    progress_callback: F,
    is_cancled: Arc<RwLock<bool>>,
) -> anyhow::Result<()> {
    let mut append_file_spec = append_file_spec.clone();
    let mut output_file = File::create(&output_file_name)?;

    let mut src_file = File::open(&src_file_spec.path)?;
    let mut append_file = File::open(&append_file_spec.path)?;

    //文件结构：源文件字节 附加文件字节 RUSTAPPEND666E FileSpec RUSTAPPEND666S
    let total = src_file_spec.size + append_file_spec.size;
    println!(
        "源文件:{} 附加文件:{} 总大小:{}",
        get_size_str(src_file_spec.size),
        get_size_str(append_file_spec.size),
        get_size_str(total)
    );

    let mut current = 0;
    let mut total_chunks = 0;
    let mut buf = [0; 1024 * 1024];
    loop {
        let len = src_file.read(&mut buf)?;
        if len == 0 {
            break;
        }
        output_file.write_all(&buf[0..len])?;
        current += len;
        total_chunks += 1;

        // 每10MB通知进度，并检查是否取消当前操作
        if let (true, Ok(canceled)) = (total_chunks % 10 == 0, is_cancled.read()) {
            let progress = ((current as f64 / total as f64) * 100.) as i32;
            progress_callback(progress);
            // println!("current={} total={} progress={progress}", get_size_str(current as u64), get_size_str(total));
            if *canceled {
                return Err(anyhow!("操作取消！"));
            }
        }
        if len < buf.len() {
            break;
        }
    }
    let mut append_total_size = 0;
    loop {
        let len = append_file.read(&mut buf)?;
        if len == 0 {
            break;
        }
        append_total_size += len;
        output_file.write_all(&buf[0..len])?;
        current += len;
        total_chunks += 1;

        // 每10MB通知进度，并检查是否取消当前操作
        if let (true, Ok(canceled)) = (total_chunks % 10 == 0, is_cancled.read()) {
            let progress = ((current as f64 / total as f64) * 100.) as i32;
            // println!("current={} total={} progress={progress}", get_size_str(current as u64), get_size_str(total));
            progress_callback(progress);
            if *canceled {
                return Err(anyhow!("操作取消！"));
            }
        }

        if len < buf.len() {
            break;
        }
    }

    append_file_spec.size = append_total_size as u64;
    let append_file_spec_data = bincode::encode_to_vec(&append_file_spec, config::standard())?;

    output_file.write_all(END_BYTES.as_bytes())?;
    output_file.write_all(&append_file_spec_data)?;
    output_file.write_all(START_BYTES.as_bytes())?;

    progress_callback(100);
    Ok(())
}

fn get_size_str(size: u64) -> String {
    Byte::from_bytes(size as u128)
        .get_appropriate_unit(false)
        .to_string()
}

/// # 保存文件和附件
///
/// 参数:
/// * `src_path`: 源文件路径
/// * `output_file`: 提取到的路径
/// * `start_offset`: 开始的字节位置
/// * `end_offset`: 结束的字节位置
/// * `progress_callback`: 进度回调函数
/// * `is_cancled`: 读写锁，用来检测操作是否取消
pub(crate) fn extract_file<F: Fn(i32)>(
    src_path: &str,
    output_file: &str,
    start_offset: u64,
    end_offset: u64,
    progress_callback: F,
    is_cancled: Arc<RwLock<bool>>,
) -> anyhow::Result<()> {
    let mut output_file = File::create(&output_file)?;

    let meta = fs::metadata(&src_path)?;
    println!("源文件信息 大小:{}", meta.file_size());

    let src_file = File::open(&src_path)?;

    let total = end_offset - start_offset;

    let mut current = 0;
    let mut total_chunks = 0;
    let mut buf = [0; 1024 * 1024];
    let mut offset = start_offset;
    println!(
        "开始提取附件start_offset={offset} end_offset={}",
        end_offset
    );
    loop {
        let mut len = src_file.seek_read(&mut buf, offset)?;
        if len == 0 {
            println!("读到的字节为0!");
            break;
        }
        if current + len <= total as usize {
            // println!("写入了{:?}", &buf[0..len]);
            output_file.write_all(&buf[0..len])?;
        } else {
            len = total as usize - current;
            // println!("写入了{:?}", &buf[0..len]);
            output_file.write_all(&buf[0..len])?;
        }
        current += len;
        total_chunks += 1;
        offset += len as u64;

        // 每10MB通知进度，并检查是否取消当前操作
        if let (true, Ok(canceled)) = (total_chunks % 10 == 0, is_cancled.read()) {
            let progress = ((current as f64 / total as f64) * 100.) as i32;
            // println!("current={} total={} progress={progress}", get_size_str(current as u64), get_size_str(total));
            progress_callback(progress);
            if *canceled {
                return Err(anyhow!("操作取消！"));
            }
        }
        if len < buf.len() {
            println!("读取长度小于缓冲区长度！");
            break;
        }
    }
    println!("文件提取结束 截止offset={offset} 写入长度:{current}");

    progress_callback(100);
    Ok(())
}