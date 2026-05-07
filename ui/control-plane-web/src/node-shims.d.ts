declare module "node:http" {
  const http: {
    createServer(handler: (req: any, res: any) => void): {
      listen(port: number, host: string, callback?: () => void): void;
    };
  };
  export default http;
}

declare module "node:fs/promises" {
  export function readFile(path: any, encoding: string): Promise<string>;
}

declare const process: {
  env: Record<string, string | undefined>;
};
