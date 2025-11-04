import NavBar from "./components/NavBar";

export default function Layout(props) {
  return (
    <>
      <NavBar />
      <main>{props.children}</main>{}
    </>
  );
}
